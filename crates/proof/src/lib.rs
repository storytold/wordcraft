//! WordCraft proofing, per language. A run's language (its BCP 47 tag) picks the tools through
//! [`ProofLang`]; the `*_in` functions take it, the older English-only ones remain.
//!
//! - **English** ([`ProofLang::En`]; also text with no language): spelling against the
//!   public-domain Moby word list (~160k words, the same list that drives hyphenation) plus our
//!   supplement of modern words it lacks (`assets/spelling/en-extra.txt`: `desktop`, `email`,
//!   `download`…), with inflections (`-s`, `-es`, `-ed`, `-ing`, `-ly`, `'s`…), a user dictionary and ignore lists;
//!   suggestions by edit distance. Grammar checks that are cheap and reliable: repeated words,
//!   `a`/`an`, capital letter after a sentence end, spaces before punctuation, doubled spaces.
//! - **Polish** ([`ProofLang::Pl`], module [`pl`]): spelling against the Hunspell `pl_PL`
//!   dictionary installed on the system ([`hunspell`]; not bundled, so without it Polish has no
//!   spelling marks), read by the affix engine ([`affix`]), suggestions tuned for Polish
//!   (diacritics, `ż`/`rz`, `u`/`ó`, missing spaces), and Polish grammar rules with Polish
//!   messages.
//! - **Bulgarian** ([`ProofLang::Bg`], module [`bg`]): hyphenation by the Bulgarian syllable
//!   rules; no spelling or grammar marks yet.
//! - **Portuguese** ([`ProofLang::Pt`], module [`pt`]): spelling against the Hunspell `pt_BR`
//!   dictionary installed on the system (VERO and friends; not bundled), suggestions tuned for
//!   Portuguese (missing accents, `seje`/`seja` confusions, missing spaces), conservative
//!   grammar rules with Portuguese messages, and `hyph-utf8`'s Portuguese patterns.
//! - **Italian** ([`ProofLang::It`], module [`it`]): spelling against the Hunspell `it_IT`
//!   dictionary installed on the system (not bundled — the upstream file is GPL-licensed),
//!   suggestions tuned for Italian (missing accents, `REP` confusions, missing spaces),
//!   conservative grammar rules with Italian messages, and `hyph-utf8`'s Italian patterns
//!   (Beccari, MIT option).
//! - **Russian** ([`ProofLang::Ru`], module [`ru`]): spelling against the Hunspell `ru_RU`
//!   dictionary installed on the system (not bundled either), with `ё` optional and hyphenated
//!   words (`кто-то`, `по-русски`, `северо-западный`) understood, suggestions tuned for Russian
//!   (`е`/`ё`, `жи`/`ши`, `тся`/`ться`, voiced and voiceless pairs, missing spaces), Russian
//!   grammar rules with Russian messages, and Russian word division.
//! - **Czech** ([`ProofLang::Cs`], module [`cs`]): the installed Hunspell `cs_CZ` dictionary
//!   (not bundled), dictionary-driven suggestions and generic checks with Czech messages.
//!   Hyphenation uses the existing English fallback; no Czech patterns are bundled.
//! - **Indonesian** ([`ProofLang::Id`], module [`id`]): the installed Hunspell `id_ID`
//!   dictionary (not bundled), dictionary-driven suggestions and generic checks with Indonesian
//!   messages. Hyphenation uses the existing English fallback.
//! - **Spanish** ([`ProofLang::Es`], [`ProofLang::EsMx`], [`ProofLang::Es419`], [`ProofLang::EsUy`],
//!   module [`es`]): each variety's installed Hunspell dictionary (`es_ES`, `es_MX`, `es_419`,
//!   `es_UY`, with fallbacks; not bundled), dictionary-driven suggestions, Uruguayan `vos` forms,
//!   and generic checks plus the opening `¿`/`¡` with Spanish messages. Hyphenation uses the
//!   existing English fallback.
//! - **German** ([`ProofLang::De`]): hyphenation by `hyph-utf8`'s German patterns (reformed
//!   spelling, MIT); no spelling or grammar marks yet.
//! - **Other languages** ([`ProofLang::Other`]): no spelling or grammar marks, like Word without
//!   the language's proofing tools.
//! - **Hyphenation** ([`hyphen`]): dictionary, Liang patterns (English, Polish, Portuguese,
//!   Italian, German), Bulgarian syllable rules, Russian word-division rules, heuristic.
//!
//! [`check_text`] checks a paragraph whose runs are in different languages.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod affix;
pub mod bg;
pub mod cs;
pub mod dict;
pub mod en_variants;
pub mod es;
pub mod external;
pub mod hr;
pub mod hunspell;
pub mod hyphen;
pub mod id;
pub mod it;
pub mod lang;
pub mod patterns;
pub mod pl;
pub mod pt;
pub mod ru;
pub mod tr;
pub mod vi;

use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

/// What the spelling check skips (File › Options › Proofing). Word's defaults: all on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Options {
    /// Words in capitals (`NASA`, `WORDCRAFT`) are taken as correct.
    pub ignore_uppercase: bool,
    /// Words containing digits (`B2B`, `mp3`) are taken as correct.
    pub ignore_numbers: bool,
    /// Web and email addresses and file paths (`https://…`, `a@b.org`, `www.…`) are skipped.
    pub ignore_internet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { ignore_uppercase: true, ignore_numbers: true, ignore_internet: true }
    }
}

pub use lang::ProofLang;

/// A problem found in text: byte range, kind, message and suggestions.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub kind: IssueKind,
    /// In the language of the text it was found in (Polish rules explain in Polish).
    pub message: String,
    pub suggestions: Vec<String>,
    /// The language the text was checked in: spelling suggestions come from its dictionary
    /// ([`suggest_in`]).
    pub lang: ProofLang,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    Spelling,
    Grammar,
}

fn user_words() -> &'static RwLock<HashSet<String>> {
    static U: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    U.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Bumped whenever the user dictionary changes, so cached proofing results can be dropped.
static DICTIONARY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The key a word has in the user dictionary and the Ignore All list: lower case and composed
/// (NFC), as spelling looks words up ([`composed`]). A word added, ignored, removed or loaded
/// decomposed (`zqprue\u{301}ba`) is then the same entry as its composed form (`zqpruéba`), and
/// the other way round (#3562 review).
fn dictionary_key(w: &str) -> String {
    composed(&w.to_lowercase()).into_owned()
}

/// Add a word to the user dictionary ("Add to Dictionary"). It applies to every language.
pub fn add_word(w: &str) {
    if user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(dictionary_key(w)) {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Take a word out of the user dictionary (File › Options › Proofing). Returns whether it was there.
pub fn remove_word(w: &str) -> bool {
    let removed = user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&dictionary_key(w));
    if removed {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    removed
}

fn ignored_words() -> &'static RwLock<HashSet<String>> {
    static I: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    I.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Ignore a word everywhere until the app closes ("Ignore All"). Unlike [`add_word`] it isn't
/// part of the user dictionary: it is never listed, saved or loaded, as in Word.
pub fn ignore_word(w: &str) {
    if ignored_words().write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(dictionary_key(w)) {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Search `dir` for Hunspell dictionaries before the system locations (File › Options ›
/// Proofing › Dictionaries folder, #1063); `None` clears it. The dictionaries load again from
/// the new search path, and text is checked again.
pub fn set_dictionaries_folder(dir: Option<std::path::PathBuf>) {
    hunspell::set_dictionary_dir(dir);
    pl::reload_dictionary();
    pt::reload_dictionary();
    it::reload_dictionary();
    hr::reload_dictionary();
    tr::reload_dictionary();
    ru::reload_dictionary();
    cs::reload_dictionary();
    id::reload_dictionary();
    es::reload_dictionary();
    DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Each language that reads an installed Hunspell dictionary, with the `.dic` file found for it
/// now (`None`: not installed, so the language has no spelling marks).
pub fn installed_dictionaries() -> Vec<(ProofLang, Option<std::path::PathBuf>)> {
    vec![
        (ProofLang::Pl, pl::installed_path()),
        (ProofLang::Pt, pt::installed_path()),
        (ProofLang::It, it::installed_path()),
        (ProofLang::Hr, hr::installed_path()),
        (ProofLang::Tr, tr::installed_path()),
        (ProofLang::Ru, ru::installed_path()),
        (ProofLang::Cs, cs::installed_path()),
        (ProofLang::Id, id::installed_path()),
        (ProofLang::Es, es::installed_path(ProofLang::Es)),
        (ProofLang::EsMx, es::installed_path(ProofLang::EsMx)),
        (ProofLang::Es419, es::installed_path(ProofLang::Es419)),
        (ProofLang::EsUy, es::installed_path(ProofLang::EsUy)),
    ]
}

/// Why the dictionary found for `lang` failed to load (unsupported encoding, too large, a parse
/// error), if it was read and failed. Doesn't read anything itself: a dictionary nobody has
/// checked yet reports `None` (#2830).
pub fn dictionary_load_error(lang: ProofLang) -> Option<String> {
    let key = match lang {
        ProofLang::Pl => "pl_PL",
        ProofLang::Pt => "pt_BR",
        ProofLang::It => "it_IT",
        ProofLang::Hr => "hr_HR",
        ProofLang::Tr => "tr_TR",
        ProofLang::Id => "id_ID",
        ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy => es::dictionary_name(lang),
        _ => return None,
    };
    hunspell::load_error(key).map(|(_, reason)| reason)
}

/// A number that changes whenever a word is added to or removed from the user dictionary, or the
/// dictionaries folder changes.
pub fn dictionary_generation() -> u64 {
    DICTIONARY_GENERATION.load(Ordering::Relaxed)
}

/// Is `lower` (a lower-case word) in the user dictionary or ignored? It is looked up composed, as
/// the entries are kept ([`dictionary_key`]).
pub(crate) fn in_user_dictionary(lower: &str) -> bool {
    let key = composed(lower);
    user_words().read().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&*key)
        || ignored_words().read().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&*key)
}

/// Whether `w` is in the user dictionary (added or ignored), any case, edge punctuation trimmed.
fn is_user_word(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’').to_lowercase();
    !w.is_empty() && in_user_dictionary(&w)
}

/// Words in the user dictionary.
pub fn user_dictionary() -> Vec<String> {
    let mut v: Vec<String> = user_words().read().unwrap_or_else(std::sync::PoisonError::into_inner).iter().cloned().collect();
    v.sort();
    v
}

/// Longest custom-dictionary word kept, in characters (mirrors `file.options` validation).
pub const MAX_CUSTOM_WORD_CHARS: usize = 64;
/// Most custom-dictionary words read from one file (caps a hostile file).
const MAX_CUSTOM_WORDS: usize = 100_000;
/// Largest custom-dictionary file read, in bytes (caps a hostile file).
const MAX_CUSTOM_FILE_BYTES: u64 = 1_000_000;

/// `w` as a custom-dictionary word: trimmed and lower-cased, or `None` when it isn't a single
/// word of at most [`MAX_CUSTOM_WORD_CHARS`] characters (empty, spaces, control characters, or
/// bytes that weren't UTF-8).
/// Adding and loading both use it, so a word that can be added is never dropped on the next load.
pub fn custom_word(w: &str) -> Option<String> {
    let w = w.trim();
    if w.is_empty()
        || w.chars().count() > MAX_CUSTOM_WORD_CHARS
        || w.chars().any(|c| c.is_whitespace() || c.is_control() || c == char::REPLACEMENT_CHARACTER)
    {
        return None;
    }
    Some(dictionary_key(w))
}

/// The valid words of the custom-dictionary file at `path`, reading at most
/// [`MAX_CUSTOM_FILE_BYTES`] of it and keeping at most [`MAX_CUSTOM_WORDS`]. A missing or
/// unreadable file reads as no words.
fn read_dictionary_file(path: &std::path::Path) -> Vec<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    // `take` bounds the read itself, so a huge file is never read whole.
    if file.take(MAX_CUSTOM_FILE_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    // Lossy: a cut multi-byte tail becomes U+FFFD and is skipped by validation below,
    // instead of discarding the whole file.
    String::from_utf8_lossy(&bytes).lines().filter_map(custom_word).take(MAX_CUSTOM_WORDS).collect()
}

/// Read custom-dictionary words from a file into the user dictionary (one word per line;
/// blank lines are skipped). A missing file reads as empty; a hostile file is capped,
/// never a panic and never an error that stops startup. Returns the words added.
pub fn load_user_dictionary(path: &std::path::Path) -> Vec<String> {
    let words = read_dictionary_file(path);
    let mut set = user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut added = Vec::new();
    for w in words {
        if set.len() >= MAX_CUSTOM_WORDS {
            break;
        }
        if set.insert(w.clone()) {
            added.push(w);
        }
    }
    if !added.is_empty() {
        DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    added
}

/// Write the user dictionary to a file (one word per line, sorted), creating its folder.
///
/// Words another process (a second window, the MCP bridge) saved to the file since it was loaded
/// are merged in first, so neither loses the other's words; `removed` (words just taken out) stay
/// out. The file is replaced atomically: written beside it, then renamed over it. Errors (a
/// read-only folder, a full disk) go back to the caller; the in-memory words are kept.
pub fn save_user_dictionary(path: &std::path::Path, removed: &[String]) -> std::io::Result<()> {
    let removed: HashSet<String> = removed.iter().map(|w| dictionary_key(w)).collect();
    let on_disk = read_dictionary_file(path);
    {
        let mut set = user_words().write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        for w in on_disk {
            if set.len() >= MAX_CUSTOM_WORDS {
                break;
            }
            if !removed.contains(&w) && set.insert(w) {
                changed = true;
            }
        }
        for w in &removed {
            changed |= set.remove(w);
        }
        if changed {
            DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
    }
    let mut out = String::new();
    for w in user_dictionary() {
        out.push_str(&w);
        out.push('\n');
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "custom-dictionary.txt".into());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::write(&tmp, out).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Common words the source list lacks (modern vocabulary, contractions).
const COMMON: &str = "a an the and or but if of to in on at by for with from as is am are was were be been being it its we us our you your he him his she her they them their i me my mine this that these those there here what which who whom whose when where why how all any both each few more most other some such no nor not only own same so than too very can will just should now do does did done has have had having may might must shall would could up down out over under again further then once off into onto upon about above below between through during before after until while because per via vs etc ok yes yeah oh ah hi hey bye one two three four five six seven eight nine ten first last next new old get got go goes went gone see saw seen say said make made take took come came give gave know knew think thought look want need use used find found tell told ask asked feel felt try left keep let put mean set run show turn move live play pay hear hold bring begin seem help talk start might";

/// Our supplement of common words the Moby list lacks (`desktop`, `email`, `download`…): one
/// lowercase base form per line, `#` comments. Inflections come from [`known_core`].
const EXTRA_EN: &str = include_str!("../../../assets/spelling/en-extra.txt");

fn extra_words() -> &'static HashSet<&'static str> {
    static E: OnceLock<HashSet<&str>> = OnceLock::new();
    E.get_or_init(|| EXTRA_EN.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect())
}

/// A base form in the dictionary or the supplement.
fn base_known(d: &dict::Dictionary, w: &str) -> bool {
    d.get(w).is_some() || extra_words().contains(w)
}

fn known_core(w: &str) -> bool {
    let d = dict::Dictionary::en_us();
    if base_known(d, w) || COMMON.split(' ').any(|c| c == w) {
        return true;
    }
    if in_user_dictionary(w) {
        return true;
    }
    // Inflections.
    let stems: [(&str, &[&str]); 13] = [
        ("'s", &[""]),
        ("s", &[""]),
        ("es", &["", "e"]),
        ("ies", &["y"]),
        ("ed", &["", "e"]),
        ("ied", &["y"]),
        ("ing", &["", "e"]),
        ("ly", &["", "le"]),
        ("ily", &["y"]),
        // specifically → specific (#1694): only -ic adjectives, so "desktopally" stays wrong.
        ("ically", &["ic"]),
        ("er", &["", "e"]),
        ("est", &["", "e"]),
        ("ness", &[""]),
    ];
    for (suf, adds) in stems {
        if let Some(stem) = w.strip_suffix(suf)
            && stem.len() >= 2
        {
            for a in adds {
                let s = format!("{stem}{a}");
                if base_known(d, &s) {
                    return true;
                }
            }
            // Doubled consonant: running → run, stopped → stop.
            let b = stem.as_bytes();
            if (suf == "ing" || suf == "ed" || suf == "er" || suf == "est")
                && b.len() >= 3
                && b.last() == b.get(b.len() - 2)
                && let Some(s) = stem.get(..stem.len() - 1)
                && base_known(d, s)
            {
                return true;
            }
        }
    }
    // Prefixes.
    for pre in ["un", "re", "non", "pre", "over", "under", "co", "mis"] {
        if let Some(rest) = w.strip_prefix(pre)
            && rest.len() >= 3
            && base_known(d, rest)
        {
            return true;
        }
    }
    false
}

/// Is there a proofing dictionary for this BCP 47 language tag? `None` (and empty tags) mean the
/// document default, which is en-US. Runs in a language without one are skipped (#100) instead
/// of flagging every word against another language's dictionary; see [`ProofLang::from_tag`].
pub fn has_dictionary(lang: Option<&str>) -> bool {
    ProofLang::from_tag(lang).is_proofed()
}

/// The Hunspell dictionary name (`pl_PL`, `it_IT`) a proofing language needs but doesn't have
/// installed, for a status-bar hint (#1167). `None` when the language's dictionary is available,
/// or when it needs none (English has a built-in word list; unproofed languages aren't offered
/// one). Reads the dictionary once (it is cached afterwards).
pub fn missing_dictionary(lang: Option<&str>) -> Option<&'static str> {
    let lang = ProofLang::from_tag(lang);
    dictionary_needed(lang, dictionary_loaded(lang)?)
}

/// The Hunspell name, `.dic` file and reason of a dictionary that is installed for this language
/// but couldn't be read (unsupported `SET` encoding, too large, a parse error). `None` when it
/// loaded, or when no file exists (then [`missing_dictionary`] names it). Reads the dictionary
/// once (it is cached afterwards).
pub fn unreadable_dictionary(lang: Option<&str>) -> Option<(&'static str, std::path::PathBuf, String)> {
    let lang = ProofLang::from_tag(lang);
    let name = dictionary_needed(lang, dictionary_loaded(lang)?)?;
    hunspell::load_error(name).map(|(file, reason)| (name, file, reason))
}

/// Did the Hunspell dictionary of `lang` load? `None` for languages that use none (English).
fn dictionary_loaded(lang: ProofLang) -> Option<bool> {
    Some(match lang {
        ProofLang::Pl => pl::has_dictionary(),
        ProofLang::Pt => pt::has_dictionary(),
        ProofLang::It => it::has_dictionary(),
        ProofLang::Hr => hr::has_dictionary(),
        ProofLang::Tr => tr::has_dictionary(),
        ProofLang::Ru => ru::has_dictionary(),
        ProofLang::Cs => cs::has_dictionary(),
        ProofLang::Id => id::has_dictionary(),
        l if l.is_spanish() => es::has_dictionary(l),
        _ => return None,
    })
}

/// The Hunspell name `lang` needs when `installed` is false (the first name its module looks for;
/// a Spanish variety's is its own: `es_ES`, `es_MX`, `es_419`, `es_UY`).
fn dictionary_needed(lang: ProofLang, installed: bool) -> Option<&'static str> {
    match lang {
        _ if installed => None,
        ProofLang::Pl => Some("pl_PL"),
        ProofLang::Pt => Some("pt_BR"),
        ProofLang::It => Some("it_IT"),
        ProofLang::Hr => Some("hr_HR"),
        ProofLang::Tr => Some("tr_TR"),
        ProofLang::Ru => Some("ru_RU"),
        ProofLang::Cs => Some("cs_CZ"),
        ProofLang::Id => Some("id_ID"),
        l if l.is_spanish() => Some(es::dictionary_name(l)),
        _ => None,
    }
}

/// Is `word` spelled correctly in the run's proofing language? A language without a dictionary
/// can't be checked, so every word counts as correct (no red underlines for it).
pub fn is_correct_for(word: &str, lang: Option<&str>) -> bool {
    is_correct_in(word, ProofLang::from_tag(lang))
}

/// Is `word` spelled correctly? Numbers, single letters, ALL-CAPS acronyms, URLs and words with
/// digits are accepted.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(word, &Options::default())
}

/// The words every language's check accepts under `opts`: single letters, words without
/// letters, words with digits (Ignore words that contain numbers), words in capitals
/// (Ignore words in UPPERCASE) and words in the user dictionary.
fn accepted_by_options(word: &str, opts: &Options) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').trim_matches('\'');
    w.chars().count() <= 1
        || !w.chars().any(char::is_alphabetic)
        || (opts.ignore_numbers && w.chars().any(|c| c.is_ascii_digit()))
        || (opts.ignore_uppercase && w.chars().all(|c| !c.is_lowercase()))
        // The whole word: the languages split a hyphenated word into its parts, and an added
        // `Unigranrio-RJ` is none of them (#2082).
        || in_user_dictionary(&w.to_lowercase())
}

/// [`is_correct`] with the File › Options › Proofing choices.
pub fn is_correct_with(word: &str, opts: &Options) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    let w = w.trim_matches('\'');
    if accepted_by_options(w, opts) {
        return true;
    }
    if !w.is_ascii() && !w.chars().any(|c| c.is_ascii_alphabetic()) {
        return true; // other scripts: no dictionary
    }
    let lower = w.to_lowercase().replace('’', "'");
    if known_core(&lower) {
        return true;
    }
    // Hyphenated compounds: each part.
    if lower.contains('-') {
        return lower.split('-').all(|p| p.is_empty() || known_core(p));
    }
    false
}

/// Spelling suggestions for a misspelled word (best first, at most `max`).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let lower = word.to_lowercase();
    let n = lower.chars().count();
    if n == 0 || n > 40 {
        return Vec::new();
    }
    let d = dict::Dictionary::en_us();
    let mut scored: Vec<(usize, i64, String)> = Vec::new();
    let target: Vec<char> = lower.chars().collect();
    for w in d.iter().map(|(w, _)| w).chain(extra_words().iter().copied()) {
        let wn = w.chars().count();
        if wn + 2 < n || wn > n + 2 {
            continue;
        }
        let dist = damerau(&target, w);
        if dist <= 2 {
            let same_first = w.chars().next() == target.first().copied();
            scored.push((dist, if same_first { 0 } else { 1 }, w.to_string()));
        }
    }
    scored.sort_by(|a, b| (a.0, a.1, a.2.len().abs_diff(n)).cmp(&(b.0, b.1, b.2.len().abs_diff(n))).then(a.2.cmp(&b.2)));
    scored.into_iter().take(max).map(|(_, _, w)| match_case(word, w)).collect()
}

/// `w` (lower case) in the case of `word`: all capitals, an initial capital, or as it is.
fn match_case(word: &str, w: String) -> String {
    let cap = word.chars().next().is_some_and(char::is_uppercase);
    let upper = word.chars().all(|c| !c.is_lowercase());
    if upper && word.chars().count() > 1 {
        w.to_uppercase()
    } else if cap {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(w)
    } else {
        w
    }
}

/// `lower` in the spelling `lang` (an English) wants, when it or a part of a hyphenated compound
/// is the other dialect's spelling (`colour-blind` → `color-blind` in American English).
fn english_preferred(lower: &str, lang: ProofLang) -> Option<String> {
    let mut changed = false;
    let parts: Vec<String> = lower
        .split('-')
        .map(|p| match en_variants::preferred(p, lang) {
            Some(n) => {
                changed = true;
                n
            }
            None => p.to_string(),
        })
        .collect();
    changed.then(|| parts.join("-"))
}

/// Is `word` spelled right in the English `lang` ([`ProofLang::En`], [`ProofLang::EnGb`],
/// [`ProofLang::EnCa`])? The shared word list's words, less the other dialect's spellings
/// (`colour` in American English, `color` in British), plus this dialect's spellings of the
/// list's words ([`en_variants`]). The user dictionary overrules both.
fn is_correct_english(word: &str, lang: ProofLang, opts: &Options) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').trim_matches('\'');
    if accepted_by_options(w, opts) {
        return true;
    }
    let lower = w.to_lowercase().replace('’', "'");
    if in_user_dictionary(&lower) {
        return true;
    }
    let parts: Vec<&str> = lower.split('-').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| en_variants::is_foreign(p, lang)) {
        return false;
    }
    is_correct_with(word, opts) || (!parts.is_empty() && parts.iter().all(|p| known_core(p) || en_variants::known_variant(p, lang, known_core)))
}

/// Spelling suggestions in the English `lang`: this dialect's spelling first when `word` is the
/// other dialect's, then the shared suggestions in this dialect's spelling.
fn suggest_english(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let lower = word.to_lowercase().replace('’', "'");
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = english_preferred(&lower, lang) {
        out.push(match_case(word, p));
    }
    for s in suggest(word, max.saturating_add(2)) {
        let s = english_preferred(&s.to_lowercase(), lang).map_or(s, |p| match_case(word, p));
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.truncate(max);
    out
}

/// Optimal string alignment distance, early-exit at 3.
fn damerau(a: &[char], b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut prev2 = vec![0usize; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        if let Some(c) = cur.get_mut(0) {
            *c = i;
        }
        let mut row_min = i;
        for j in 1..=m {
            let (ai, bj) = (a.get(i - 1), b.get(j - 1));
            let cost = usize::from(ai != bj);
            let mut v = (prev.get(j).copied().unwrap_or(9) + 1)
                .min(cur.get(j - 1).copied().unwrap_or(9) + 1)
                .min(prev.get(j - 1).copied().unwrap_or(9) + cost);
            if i > 1 && j > 1 && a.get(i - 1) == b.get(j - 2) && a.get(i - 2) == b.get(j - 1) {
                v = v.min(prev2.get(j - 2).copied().unwrap_or(9) + 1);
            }
            if let Some(c) = cur.get_mut(j) {
                *c = v;
            }
            row_min = row_min.min(v);
        }
        if row_min > 2 {
            return 3;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(m).copied().unwrap_or(9)
}

/// Words in `text`: (byte start, byte end). Combining marks belong to the word they follow, so a
/// word typed decomposed (`tambie\u{301}n`, NFD) is one word, as its precomposed form is.
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-' || unicode_normalization::char::is_combining_mark(c);
    for (i, c) in text.char_indices() {
        match (start, is_w(c)) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    // Trim surrounding apostrophes/hyphens.
    out.into_iter()
        .filter_map(|(a, b)| {
            let w = text.get(a..b)?;
            let lead = w.len() - w.trim_start_matches(['\'', '’', '-']).len();
            let trail = w.len() - w.trim_end_matches(['\'', '’', '-']).len();
            (a + lead < b - trail).then_some((a + lead, b - trail))
        })
        .collect()
}

/// Spelling issues in a paragraph of English text (no suggestions; ask [`suggest`] on demand).
pub fn check_spelling(text: &str) -> Vec<Issue> {
    check_spelling_with(text, &Options::default())
}

/// [`check_spelling`] with the File › Options › Proofing choices.
pub fn check_spelling_with(text: &str, opts: &Options) -> Vec<Issue> {
    spelling_issues(text, |_| ProofLang::En, opts)
}

/// Spelling issues in a paragraph of text in `lang` (none for a language without a dictionary;
/// no suggestions: ask [`suggest_in`] on demand).
pub fn check_spelling_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    if !lang.is_proofed() {
        return Vec::new();
    }
    spelling_issues(text, |_| lang, &Options::default())
}

/// Spelling issues of every word of `text`, each word spelled in `lang(word_start)`.
///
/// The web/e-mail classification of the whitespace-delimited token around a word is computed
/// once per token and shared by every word punctuation splits out of it. Classifying per word
/// rescanned the token each time, so a long whitespace-free run of punctuation-separated words
/// cost quadratic scanning (#1335).
fn spelling_issues(text: &str, lang: impl Fn(usize) -> ProofLang, opts: &Options) -> Vec<Issue> {
    let mut token = (0, 0, false);
    words(text)
        .into_iter()
        .filter_map(|(a, b)| {
            if a < token.0 || a >= token.1 {
                token = enclosing_token(text, a, b);
            }
            spelling_issue(text, a, b, lang(a), opts, token.2)
        })
        .collect()
}

/// The whitespace-delimited token enclosing the word at `a..b` of `text`: its byte range, and
/// whether it looks like a web or e-mail address.
fn enclosing_token(text: &str, a: usize, b: usize) -> (usize, usize, bool) {
    // The token starts after the last white-space character, whatever its width (a no-break
    // space is two bytes and an ideographic space is three).
    let ts = text.get(..a).and_then(|t| t.char_indices().rev().find(|(_, c)| c.is_whitespace())).map(|(i, c)| i + c.len_utf8()).unwrap_or(0);
    let te = text.get(b..).and_then(|t| t.find(char::is_whitespace)).map(|i| b + i).unwrap_or(text.len());
    (ts, te, text.get(ts..te).is_some_and(is_address))
}

/// Does a whitespace-delimited token look like a web or e-mail address (`https://…`, `www.…`,
/// `a@b.org`)? Spelling skips its words, and Spanish grammar its punctuation.
pub(crate) fn is_address(token: &str) -> bool {
    token.contains("://") || token.contains('@') || token.starts_with("www.")
}

/// Whether `opts` says to leave the spelling issue at `a..b` of `text` unmarked (Ignore words in
/// UPPERCASE, words with numbers, Internet and file addresses). For results from an external
/// checker, which doesn't know these options (#575).
pub fn spelling_ignored_by_options(text: &str, a: usize, b: usize, opts: &Options) -> bool {
    let Some(w) = text.get(a..b) else { return false };
    accepted_by_options(w, opts) || (opts.ignore_internet && enclosing_token(text, a, b).2)
}

/// The spelling issue of the word at `a..b` of `text`, if it is misspelled in `lang`. Words in
/// web and e-mail addresses are skipped (unless `opts` says otherwise); `internet` is the
/// classification of the word's enclosing token, from [`enclosing_token`].
fn spelling_issue(text: &str, a: usize, b: usize, lang: ProofLang, opts: &Options, internet: bool) -> Option<Issue> {
    let w = text.get(a..b)?;
    // A word with a Vietnamese-only letter (a name like Nguyễn in English text) isn't English.
    if (opts.ignore_internet && internet) || (lang.is_english() && vi::is_vietnamese_word(w)) || is_correct_in_with(w, lang, opts) {
        return None;
    }
    let message = match lang {
        ProofLang::Pl => "Możliwy błąd pisowni",
        ProofLang::Hr => "Moguća pravopisna pogreška",
        ProofLang::Pt => "Possível erro de ortografia",
        ProofLang::It => "Possibile errore di ortografia",
        ProofLang::Tr => "Olası yazım hatası",
        ProofLang::Ru => "Возможна орфографическая ошибка",
        ProofLang::Cs => "Možná pravopisná chyba",
        ProofLang::Id => "Kemungkinan salah eja",
        ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy => "Posible error ortográfico",
        _ => "Possible spelling mistake",
    };
    Some(Issue { start: a, end: b, kind: IssueKind::Spelling, message: message.into(), suggestions: Vec::new(), lang })
}

/// Is `word` spelled correctly in `lang`? Always true in a language without a dictionary.
pub fn is_correct_in(word: &str, lang: ProofLang) -> bool {
    is_correct_in_with(word, lang, &Options::default())
}

/// `word` composed (NFC), as the dictionaries spell their words: `tambie\u{301}n` is looked up as
/// `también`. Words already composed (all ASCII ones among them) are borrowed as they are.
fn composed(word: &str) -> std::borrow::Cow<'_, str> {
    use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};
    if word.is_ascii() || is_nfc_quick(word.chars()) == IsNormalized::Yes {
        std::borrow::Cow::Borrowed(word)
    } else {
        std::borrow::Cow::Owned(word.nfc().collect())
    }
}

/// [`is_correct_in`] with the File › Options › Proofing choices. A word with combining marks is
/// checked as its composed form ([`composed`]).
pub fn is_correct_in_with(word: &str, lang: ProofLang, opts: &Options) -> bool {
    let word = &*composed(word);
    match lang {
        ProofLang::En | ProofLang::EnGb | ProofLang::EnCa => is_correct_english(word, lang, opts),
        ProofLang::Pl => accepted_by_options(word, opts) || pl::is_correct_with_options(word, opts),
        ProofLang::Hr => accepted_by_options(word, opts) || hr::is_correct_with_options(word, opts),
        ProofLang::Pt => accepted_by_options(word, opts) || pt::is_correct_with_options(word, opts),
        ProofLang::It => accepted_by_options(word, opts) || it::is_correct_with_options(word, opts),
        ProofLang::Tr => accepted_by_options(word, opts) || tr::is_correct(word),
        ProofLang::Ru => accepted_by_options(word, opts) || ru::is_correct(word),
        ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy => {
            accepted_by_options(word, opts) || es::is_correct_with_options(word, lang, opts)
        }
        ProofLang::Cs => accepted_by_options(word, opts) || cs::is_correct_with_options(word, opts),
        ProofLang::Id => accepted_by_options(word, opts) || id::is_correct_with_options(word, opts),
        ProofLang::Bg | ProofLang::De | ProofLang::Other => true,
    }
}

/// Spelling suggestions for a word misspelled in `lang` (best first, at most `max`), composed
/// ([`composed`]) like the dictionaries' words.
pub fn suggest_in(word: &str, max: usize, lang: ProofLang) -> Vec<String> {
    let word = &*composed(word);
    match lang {
        ProofLang::En | ProofLang::EnGb | ProofLang::EnCa => suggest_english(word, max, lang),
        ProofLang::Pl => pl::suggest(word, max),
        ProofLang::Hr => hr::suggest(word, max),
        ProofLang::Pt => pt::suggest(word, max),
        ProofLang::It => it::suggest(word, max),
        ProofLang::Tr => tr::suggest(word, max),
        ProofLang::Ru => ru::suggest(word, max),
        ProofLang::Cs => cs::suggest(word, max),
        ProofLang::Id => id::suggest(word, max),
        ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy => es::suggest(word, max, lang),
        ProofLang::Bg | ProofLang::De | ProofLang::Other => Vec::new(),
    }
}

/// Grammar issues in a paragraph of text in `lang`: the English rules, the Polish ones
/// ([`pl::grammar`]), the Portuguese ones ([`pt::grammar`]), the Italian ones ([`it::grammar`]),
/// the Turkish ones ([`tr::grammar`]), Czech and Indonesian generic checks ([`cs::grammar`], [`id::grammar`]), the Spanish ones
/// ([`es::grammar`]), or none for a language WordCraft doesn't proof.
pub fn check_grammar_in(text: &str, lang: ProofLang) -> Vec<Issue> {
    grammar_in(text, lang, true)
}

/// [`check_grammar_in`] where `capital_first` says whether the first word must be capitalized
/// (false for a list item).
fn grammar_in(text: &str, lang: ProofLang, capital_first: bool) -> Vec<Issue> {
    match lang {
        ProofLang::En => grammar_en(text, capital_first),
        ProofLang::EnGb | ProofLang::EnCa => grammar_en(text, capital_first).into_iter().map(|i| Issue { lang, ..i }).collect(),
        ProofLang::Pl => pl::grammar::check_with(text, capital_first),
        ProofLang::Hr => hr::grammar::check_with(text, capital_first),
        ProofLang::Pt => pt::grammar::check_with(text, capital_first),
        ProofLang::It => it::grammar::check_with(text, capital_first),
        ProofLang::Tr => tr::grammar::check_with(text, capital_first),
        ProofLang::Ru => ru::grammar::check_with(text, capital_first),
        // Czech has no sentence-capital rule, so a list item needs no exception.
        ProofLang::Cs => cs::grammar::check(text),
        ProofLang::Id => id::grammar::check(text),
        // Spanish has no sentence-capital rule either.
        ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy => es::grammar::check(text, lang),
        ProofLang::Bg | ProofLang::De | ProofLang::Other => Vec::new(),
    }
}

/// Spelling and grammar issues in a paragraph whose text is in several languages, in order.
/// `spans` gives the language of byte ranges of `text` (in order, not overlapping); text outside
/// them is English. Each word is spelled in the language of its first letter. Each language's
/// grammar rules read the whole paragraph (so sentence boundaries are seen) but report only
/// inside that language's spans: an English quotation in a Polish paragraph gets English
/// spelling and is never told a Polish rule.
pub fn check_text(text: &str, spans: &[(Range<usize>, ProofLang)]) -> Vec<Issue> {
    check_text_with(text, spans, &Options::default(), true, false)
}

/// [`check_text`] with the File › Options › Proofing choices: what spelling skips, and whether
/// grammar is checked at all.
///
/// `list_item` marks a list paragraph: items are often fragments, so the English rule asking for
/// a capital on the paragraph's first word is off for it (later sentences are still checked).
pub fn check_text_with(text: &str, spans: &[(Range<usize>, ProofLang)], opts: &Options, grammar: bool, list_item: bool) -> Vec<Issue> {
    // Sorted, empty ones dropped, neighbours of one language merged.
    let mut sorted: Vec<&(Range<usize>, ProofLang)> = spans.iter().filter(|(r, _)| r.start < r.end).collect();
    sorted.sort_by_key(|s| s.0.start);
    let mut merged: Vec<(Range<usize>, ProofLang)> = Vec::new();
    for (r, l) in sorted {
        match merged.last_mut() {
            Some((m, ml)) if *ml == *l && m.end == r.start => m.end = r.end,
            _ => merged.push((r.clone(), *l)),
        }
    }
    // A paragraph typed in Vietnamese under the default English language is Vietnamese, which
    // has no proofing tools: no marks (Word detects the language the same way).
    let default = if vi::looks_vietnamese(text) {
        for (_, l) in merged.iter_mut().filter(|(_, l)| l.is_english()) {
            *l = ProofLang::Other;
        }
        ProofLang::Other
    } else {
        ProofLang::En
    };
    let lang_at = |i: usize| -> ProofLang {
        let k = merged.partition_point(|(r, _)| r.end <= i);
        merged.get(k).filter(|(r, _)| r.start <= i).map_or(default, |s| s.1)
    };
    let mut v: Vec<Issue> = spelling_issues(text, lang_at, opts);
    if !grammar {
        v.sort_by_key(|i| (i.start, i.end));
        return v;
    }
    let mut langs: Vec<ProofLang> = merged.iter().map(|s| s.1).collect();
    let mut covered = 0;
    for (r, _) in &merged {
        if r.start > covered {
            break;
        }
        covered = covered.max(r.end);
    }
    if covered < text.len() || merged.is_empty() {
        langs.push(default);
    }
    langs.sort();
    langs.dedup();
    for lang in langs {
        let found = grammar_in(text, lang, !list_item);
        v.extend(found.into_iter().filter(|i| lang_at(i.start) == lang));
    }
    v.sort_by_key(|i| (i.start, i.end));
    v
}

/// The external checker's issues for exactly `text` checked in `language`, if it has answered for it ([`external`]),
/// without spelling issues on words the user added or ignored. `None`: use the built-in checks.
pub fn check_external(text: &str, language: &str) -> Option<Vec<Issue>> {
    let found = external::get(text, language)?;
    Some(found.into_iter().filter(|i| i.kind != IssueKind::Spelling || !text.get(i.start..i.end).is_some_and(is_user_word)).collect())
}

/// Grammar issues in a paragraph of English text.
pub fn check_grammar(text: &str) -> Vec<Issue> {
    grammar_en(text, true)
}

/// The English rules; `capital_first` is whether the text's first word must start a sentence
/// with a capital (false for a list item, which may be a fragment).
fn grammar_en(text: &str, capital_first: bool) -> Vec<Issue> {
    let mut v = Vec::new();
    let ws = words(text);
    for pair in ws.windows(2) {
        let [(a0, b0), (a1, b1)] = pair else { continue };
        let (Some(w0), Some(w1), Some(gap)) = (text.get(*a0..*b0), text.get(*a1..*b1), text.get(*b0..*a1)) else { continue };
        if gap.trim().is_empty()
            && w0.eq_ignore_ascii_case(w1)
            && w0.chars().all(char::is_alphabetic)
            && !matches!(w0.to_lowercase().as_str(), "had" | "that" | "bye" | "so")
        {
            v.push(Issue {
                start: *a0,
                end: *b1,
                kind: IssueKind::Grammar,
                message: format!("Repeated word: \"{w1}\""),
                suggestions: vec![w0.to_string()],
                lang: ProofLang::En,
            });
        }
        if gap == " " && (w0 == "a" || w0 == "A") && w1.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c)) && !starts_consonant_sound(w1) {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"an\" before a vowel sound".into(),
                suggestions: vec![if w0 == "A" { "An" } else { "an" }.into()],
                lang: ProofLang::En,
            });
        }
        if gap == " "
            && (w0 == "an" || w0 == "An")
            && w1.chars().next().is_some_and(|c| c.is_alphabetic() && !"aeiouAEIOUhH".contains(c))
            && !w1.chars().all(char::is_uppercase)
        {
            v.push(Issue {
                start: *a0,
                end: *b0,
                kind: IssueKind::Grammar,
                message: "Use \"a\" before a consonant sound".into(),
                suggestions: vec![if w0 == "An" { "A" } else { "a" }.into()],
                lang: ProofLang::En,
            });
        }
    }
    // Space before punctuation; doubled spaces.
    for (i, _) in text.match_indices(" ,").chain(text.match_indices(" .")).chain(text.match_indices(" ;")) {
        if text.get(i + 2..).is_some_and(|r| r.starts_with('.') || r.starts_with(',')) {
            continue; // "..." or numbers like " .5"
        }
        v.push(Issue {
            start: i,
            end: i + 2,
            kind: IssueKind::Grammar,
            message: "Remove the space before the punctuation".into(),
            suggestions: vec![text.get(i + 1..i + 2).unwrap_or("").to_string()],
            lang: ProofLang::En,
        });
    }
    for (i, _) in text.match_indices("  ") {
        if i > 0 && text.get(..i).is_some_and(|t| t.ends_with(' ')) {
            continue;
        }
        v.push(Issue {
            start: i,
            end: i + 2,
            kind: IssueKind::Grammar,
            message: "Extra space".into(),
            suggestions: vec![" ".into()],
            lang: ProofLang::En,
        });
    }
    // Sentence start capital.
    let mut after_end = capital_first;
    for (a, b) in &ws {
        let Some(w) = text.get(*a..*b) else { continue };
        if after_end && w.chars().next().is_some_and(char::is_lowercase) && w.chars().all(char::is_alphabetic) {
            let fixed: String = w.chars().next().map(|f| f.to_uppercase().chain(w.chars().skip(1)).collect()).unwrap_or_default();
            v.push(Issue {
                start: *a,
                end: *b,
                kind: IssueKind::Grammar,
                message: "Capitalize the first word of a sentence".into(),
                suggestions: vec![fixed],
                lang: ProofLang::En,
            });
        }
        let tail = text.get(*b..).unwrap_or("");
        let punctuation = tail.trim_start_matches(['"', '”', ')', '\'']);
        // Two or more dots are an ellipsis, not a full stop ending a sentence.
        // A Unicode ellipsis (U+2026) does not match sentence-ending punctuation either.
        after_end = matches!(punctuation.chars().next(), Some('.' | '!' | '?'))
            && !punctuation.starts_with("..")
            && punctuation.trim_start_matches(['.', '!', '?', '"', '”', ')']).starts_with(' ');
    }
    v.sort_by_key(|i| i.start);
    v
}

fn starts_consonant_sound(w: &str) -> bool {
    let l = w.to_lowercase();
    ["uni", "use", "usu", "eu", "one", "once", "ubiq", "uti", "ure", "uro"].iter().any(|p| l.starts_with(p))
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_dictionary_is_named_only_for_hunspell_languages_without_one() {
        assert_eq!(dictionary_needed(ProofLang::Pl, false), Some("pl_PL"));
        assert_eq!(dictionary_needed(ProofLang::Pt, false), Some("pt_BR"));
        assert_eq!(dictionary_needed(ProofLang::Pl, true), None);
        assert_eq!(dictionary_needed(ProofLang::Pt, true), None);
        // Every Hunspell language is named by the first name its module looks for (#3150).
        for (l, name) in [
            (ProofLang::It, "it_IT"),
            (ProofLang::Hr, "hr_HR"),
            (ProofLang::Tr, "tr_TR"),
            (ProofLang::Ru, "ru_RU"),
            (ProofLang::Cs, "cs_CZ"),
            (ProofLang::Id, "id_ID"),
            // #3360: each Spanish variety names its own.
            (ProofLang::Es, "es_ES"),
            (ProofLang::EsMx, "es_MX"),
            (ProofLang::Es419, "es_419"),
            (ProofLang::EsUy, "es_UY"),
        ] {
            assert_eq!(dictionary_needed(l, false), Some(name), "{l:?}");
            assert_eq!(dictionary_needed(l, true), None, "{l:?}");
        }
        // Every proofed language other than English has a Hunspell dictionary to report.
        for l in ProofLang::PROOFED {
            let english = matches!(l, ProofLang::En | ProofLang::EnGb | ProofLang::EnCa);
            assert_eq!(dictionary_needed(l, false).is_some(), !english, "{l:?}");
        }
        for l in [ProofLang::En, ProofLang::Bg, ProofLang::De, ProofLang::Other] {
            assert_eq!(dictionary_needed(l, false), None);
        }
        // Installed dictionaries and the built-in English list: no hint.
        pl::test_dict::install();
        pt::test_dict::install();
        es::test_dict::install();
        for tag in [Some("pl-PL"), Some("pt-BR"), Some("es-ES"), Some("es-MX"), Some("es-419"), Some("es-UY"), Some("en-US"), Some("fr-FR"), None] {
            assert_eq!(missing_dictionary(tag), None, "{tag:?}");
        }
    }

    use super::*;

    /// Held by tests that swap or rely on the shared Spanish dictionary, so a folder reset can't land mid-test.
    static DICTIONARY_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// #1063: the dictionaries folder is searched before the system locations; the dictionaries
    /// in it are reported and used, and cached checks are dropped. The folder holds the same test
    /// dictionaries the other tests install, so tests running meanwhile see the same words; it
    /// stays set for the rest of the run for that reason.
    #[test]
    fn dictionaries_folder_is_searched_first() {
        let _guard = DICTIONARY_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("wc-proof-dictionaries-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, aff, dic) in [
            ("pl_PL", pl::test_dict::AFF, pl::test_dict::DIC),
            ("pt_BR", pt::test_dict::AFF, pt::test_dict::DIC),
            ("it_IT", it::test_dict::AFF, it::test_dict::DIC),
            ("hr_HR", hr::test_dict::AFF, hr::test_dict::DIC),
            ("ru_RU", ru::test_dict::AFF, ru::test_dict::DIC),
            ("cs_CZ", cs::test_dict::AFF, cs::test_dict::DIC),
            ("id_ID", id::test_dict::AFF, id::test_dict::DIC),
            ("es_ES", es::test_dict::AFF, es::test_dict::DIC),
            ("es_MX", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
            ("es_419", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
            ("es_UY", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
        ] {
            std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
        }
        // #3252 review: force Russian and Czech to cache whatever they find before the folder is
        // set (empty, with no folder and nothing installed on CI), so the folder change below has
        // a stale cache to actually invalidate, not just an untouched `None`.
        ru::dictionary();
        cs::dictionary();
        let before = dictionary_generation();
        set_dictionaries_folder(Some(dir.clone()));
        assert!(dictionary_generation() > before, "cached checks are dropped");
        // Turkish has no test dictionary to put in the folder; Russian, Czech, Indonesian and the Spanishes follow it.
        let found = installed_dictionaries();
        assert_eq!(found.get(4).map(|(lang, _)| *lang), Some(ProofLang::Tr));
        assert_eq!(
            found[7..],
            [
                (ProofLang::Id, Some(dir.join("id_ID.dic"))),
                (ProofLang::Es, Some(dir.join("es_ES.dic"))),
                (ProofLang::EsMx, Some(dir.join("es_MX.dic"))),
                (ProofLang::Es419, Some(dir.join("es_419.dic"))),
                (ProofLang::EsUy, Some(dir.join("es_UY.dic"))),
            ]
        );
        assert_eq!(
            found[..4],
            [
                (ProofLang::Pl, Some(dir.join("pl_PL.dic"))),
                (ProofLang::Pt, Some(dir.join("pt_BR.dic"))),
                (ProofLang::It, Some(dir.join("it_IT.dic"))),
                (ProofLang::Hr, Some(dir.join("hr_HR.dic"))),
            ]
        );
        // #3226: Russian and Czech were missing from installed_dictionaries() entirely (no
        // installed_path() in either module), so neither ever showed up in File › Options ›
        // Proofing, installed or not. Their languages, not their exact path, is what belongs
        // here: unlike the four above, installed_path() for Russian and Czech also reflects
        // whether install_dictionary() gave one directly (#3252 review), which ru's and cs's own
        // test modules do, concurrently, in tests of their own — asserting an exact path here
        // would race them for no reason this test needs to care about.
        assert_eq!(found[5..7].iter().map(|(lang, _)| *lang).collect::<Vec<_>>(), [ProofLang::Ru, ProofLang::Cs], "{found:?}");
        assert!(is_correct_in("chrząszcz", ProofLang::Pl) && !is_correct_in("chrzonszcz", ProofLang::Pl));
        assert!(is_correct_in("tenés", ProofLang::EsUy) && !is_correct_in("prueva", ProofLang::EsUy));
        assert!(is_correct_in("дом", ProofLang::Ru) && !is_correct_in("дум", ProofLang::Ru));
    }

    /// #1741: a list item may be a fragment, so its first word needs no capital in any language
    /// with the rule; a plain paragraph's does, and later sentences in the item are still checked.
    #[test]
    fn list_item_first_word_needs_no_capital() {
        let grammar = |t: &str, lang: ProofLang, list: bool| -> Vec<String> {
            check_text_with(t, &[(0..t.len(), lang)], &Options::default(), true, list)
                .into_iter()
                .filter(|i| i.kind == IssueKind::Grammar)
                .filter_map(|i| t.get(i.start..i.end).map(str::to_string))
                .collect()
        };
        for (lang, text) in [
            (ProofLang::En, "item one"),
            (ProofLang::Pl, "jabłka i gruszki"),
            (ProofLang::Hr, "jabuke i kruške"),
            (ProofLang::It, "mele e pere"),
            (ProofLang::Tr, "elma ve armut"),
            (ProofLang::Pt, "maçãs e peras"),
            (ProofLang::Ru, "яблоки и груши"),
        ] {
            let first = text.split(' ').next().unwrap();
            assert!(grammar(text, lang, true).is_empty(), "{lang:?} list item: {:?}", grammar(text, lang, true));
            assert_eq!(grammar(text, lang, false), [first], "{lang:?} plain paragraph");
        }
        assert_eq!(grammar("hello world.", ProofLang::En, false), ["hello"]);
        // Later sentences in a list item are still checked.
        assert_eq!(grammar("item one. then two", ProofLang::En, true), ["then"]);
        assert_eq!(grammar("elma ve armut. sonra kiraz", ProofLang::Tr, true), ["sonra"]);
    }

    #[test]
    fn dictionary_loads() {
        assert!(dict::Dictionary::en_us().len() > 100_000);
    }

    #[test]
    fn spelling() {
        for w in [
            "if",
            "we",
            "hello",
            "world",
            "running",
            "studios",
            "don't",
            "Thursday",
            "NASA",
            "2026",
            "co-operate",
            "unhappy",
            "colour",
            "it's",
            "stopped",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["helo", "wrold", "sentense", "recieve"] {
            assert!(!is_correct(w), "{w}");
        }
        let s = suggest("recieve", 5);
        assert!(s.contains(&"receive".to_string()), "{s:?}");
        assert!(suggest("Wrold", 3).contains(&"World".to_string()));
    }

    /// #1694: `-ically` adverbs come from their `-ic` adjective (specific → specifically).
    #[test]
    fn ically_adverbs_are_known() {
        for w in ["specifically", "scientifically", "automatically", "dramatically", "Specifically"] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["specificaly", "specifcally", "zzzically"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    /// #1694 review: only `-ic` adjectives take `-ally`; other known words (dictionary or
    /// supplement) don't.
    #[test]
    fn non_ic_words_do_not_take_ally() {
        for (base, wrong) in [("desktop", "desktopally"), ("house", "houseally"), ("final", "finalally"), ("green", "greenally")] {
            assert!(is_correct(base), "{base}");
            assert!(!is_correct(wrong), "{wrong}");
        }
    }

    /// #520: common modern words (and their inflections) are known; misspellings still flagged.
    #[test]
    fn common_modern_words() {
        for w in [
            "desktop",
            "Desktops",
            "laptop",
            "download",
            "downloaded",
            "uploading",
            "email",
            "emailing",
            "website",
            "online",
            "offline",
            "smartphones",
            "login",
            "username",
            "spreadsheet",
            "database",
            "screenshots",
            "blogging",
            "inbox",
            "workplace",
            "uploader",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["dekstop", "laptp", "dowload", "spreadshet"] {
            assert!(!is_correct(w), "{w}");
        }
        assert!(suggest("dekstop", 5).contains(&"desktop".to_string()), "{:?}", suggest("dekstop", 5));
        assert!(check_spelling("My desktop and laptop download email online.").is_empty());
    }

    /// The supplement stays a sorted list of lowercase base forms.
    /// #40: British spellings and their inflections are not flagged.
    #[test]
    fn british_spellings_are_known() {
        for w in [
            "judgement",
            "defence",
            "licence",
            "programme",
            "catalogue",
            "behaviour",
            "colour",
            "colourful",
            "colourless",
            "honour",
            "honoured",
            "honouring",
            "honours",
            "honourable",
            "rigour",
            "glamour",
            "clamour",
            "endeavour",
            "paediatric",
            "prioritise",
            "prioritised",
            "prioritising",
            "privatise",
            "privatised",
            "synthesise",
            "travelogue",
            "manoeuvrable",
        ] {
            assert!(is_correct(w), "{w}");
        }
        let text = "The programme honoured her rigour and prioritised colourful, paediatric care.";
        for lang in [ProofLang::EnGb, ProofLang::EnCa] {
            let flagged: Vec<&str> = check_spelling_in(text, lang).iter().map(|i| &text[i.start..i.end]).collect();
            let expect: &[&str] = if lang == ProofLang::EnCa { &["prioritised"] } else { &[] };
            assert_eq!(flagged, expect, "{lang:?}");
        }
        // American English flags the British spellings (#1281).
        let flagged: Vec<&str> = check_spelling(text).iter().map(|i| &text[i.start..i.end]).collect();
        assert_eq!(flagged, ["programme", "honoured", "rigour", "prioritised", "colourful", "paediatric"]);
    }

    /// Each English accepts its own spellings and flags the other dialect's, suggesting its own
    /// first, as Word does (#1281, #1045).
    #[test]
    fn english_dialects_flag_each_others_spellings() {
        let (us, gb, ca) = (ProofLang::En, ProofLang::EnGb, ProofLang::EnCa);
        // (word, right in en-US, right in en-CA, right in en-GB)
        let cases = [
            ("color", true, false, false),
            ("colour", false, true, true),
            ("colours", false, true, true),
            ("coloured", false, true, true),
            ("Colourful", false, true, true),
            ("colour's", false, true, true),
            ("honorable", true, false, false),
            ("neighbourhood", false, true, true),
            ("center", true, false, false),
            ("centre", false, true, true),
            ("centred", false, true, true),
            ("centring", false, true, true),
            ("centering", true, false, false),
            ("theater", true, false, false),
            ("theatre", true, true, true),
            ("organize", true, true, true),
            ("organise", false, false, true),
            ("organisation", false, false, true),
            ("organisations", false, false, true),
            ("realised", false, false, true),
            ("recognising", false, false, true),
            ("analyze", true, true, false),
            ("analyse", false, true, true),
            ("paralysed", false, true, true),
            ("defense", true, false, false),
            ("defence", false, true, true),
            ("licence", false, true, true),
            ("license", true, true, true),
            ("practise", false, true, true),
            ("traveled", true, false, false),
            ("travelled", true, true, true),
            ("catalog", true, false, false),
            ("catalogue", true, true, true),
            ("anaemia", false, true, true),
            ("anemia", true, true, false),
            ("tire", true, true, true),
            ("tyre", false, false, true),
            ("programme", false, true, true),
            ("program", true, true, true),
            ("meter", true, true, true),
            ("metre", false, true, true),
            ("plough", false, true, true),
            ("colour-blind", false, true, true),
            // Words spelled alike everywhere, and real misspellings, are unaffected.
            ("humorous", true, true, true),
            ("honorary", true, true, true),
            ("tour", true, true, true),
            ("four", true, true, true),
            ("programmed", true, true, true),
            ("colourr", false, false, false),
            ("colorr", false, false, false),
            ("organisationz", false, false, false),
        ];
        for (w, in_us, in_ca, in_gb) in cases {
            assert_eq!(is_correct_in(w, us), in_us, "en-US: {w}");
            assert_eq!(is_correct_in(w, ca), in_ca, "en-CA: {w}");
            assert_eq!(is_correct_in(w, gb), in_gb, "en-GB: {w}");
            assert_eq!(is_correct_for(w, Some("en-AU")), in_gb, "en-AU follows British spelling: {w}");
        }
        // The dialect's own spelling comes first, in the word's case.
        assert_eq!(suggest_in("colour", 5, us).first().map(String::as_str), Some("color"));
        assert_eq!(suggest_in("Colours", 5, us).first().map(String::as_str), Some("Colors"));
        assert_eq!(suggest_in("color", 5, gb).first().map(String::as_str), Some("colour"));
        assert_eq!(suggest_in("organisation", 5, us).first().map(String::as_str), Some("organization"));
        assert_eq!(suggest_in("organisation", 5, ca).first().map(String::as_str), Some("organization"));
        assert_eq!(suggest_in("analyze", 5, gb).first().map(String::as_str), Some("analyse"));
        assert_eq!(suggest_in("CENTER", 5, gb).first().map(String::as_str), Some("CENTRE"));
        assert_eq!(suggest_in("colour's", 5, us).first().map(String::as_str), Some("color's"));
        assert_eq!(suggest_in("colour-blind", 5, us).first().map(String::as_str), Some("color-blind"));
        // Shared suggestions come in the dialect's spelling, never the other one's.
        let gb_sugg = suggest_in("colouur", 8, gb);
        assert!(gb_sugg.contains(&"colour".to_string()) && !gb_sugg.contains(&"color".to_string()), "{gb_sugg:?}");
        let us_sugg = suggest_in("colouur", 8, us);
        assert!(us_sugg.contains(&"color".to_string()) && !us_sugg.contains(&"colour".to_string()), "{us_sugg:?}");
        // A word in the user dictionary is right in every English.
        add_word("zzcolourzz");
        assert!(is_correct_in("zzcolourzz", us));
        // Issues carry the dialect, so suggestions on demand come from the same one.
        let issues = check_text("The color is nice.", &[(0..18, gb)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!((issues[0].lang, &"The color is nice."[issues[0].start..issues[0].end]), (gb, "color"));
        let grammar = check_text("the colour is nice.", &[(0..19, gb)]);
        assert!(grammar.iter().any(|i| i.kind == IssueKind::Grammar && i.lang == gb), "{grammar:?}");
    }

    /// The words #40 (folded into #1281) reported as flagged in British English, with their
    /// inflections: right in en-GB, and in en-US flagged or not as `en-variants.txt` decides.
    #[test]
    fn british_words_reported_in_issue_40_are_right_in_en_gb() {
        let (us, gb, ca) = (ProofLang::En, ProofLang::EnGb, ProofLang::EnCa);
        // (word, right in en-US, right in en-CA, right in en-GB)
        let cases = [
            // Both spellings are right everywhere: `judgment` is also British legal usage, and
            // American dictionaries list `judgement` as a variant.
            ("judgement", true, true, true),
            ("judgements", true, true, true),
            ("judgment", true, true, true),
            ("judgments", true, true, true),
            ("defence", false, true, true),
            ("defences", false, true, true),
            ("defense", true, false, false),
            // `license` is the British verb, so only American English flags `licence`.
            ("licence", false, true, true),
            ("licences", false, true, true),
            ("license", true, true, true),
            ("licensed", true, true, true),
            ("licenses", true, true, true),
            // `program` is British for computing.
            ("programme", false, true, true),
            ("programmes", false, true, true),
            ("program", true, true, true),
            ("programs", true, true, true),
            // American English accepts both.
            ("catalogue", true, true, true),
            ("catalogues", true, true, true),
            ("catalogued", true, true, true),
            ("cataloguing", true, true, true),
            ("catalog", true, false, false),
            ("catalogs", true, false, false),
        ];
        for (w, in_us, in_ca, in_gb) in cases {
            assert_eq!(is_correct_in(w, us), in_us, "en-US: {w}");
            assert_eq!(is_correct_in(w, ca), in_ca, "en-CA: {w}");
            assert_eq!(is_correct_in(w, gb), in_gb, "en-GB: {w}");
        }
        let text = "The judgement on the defence licence put the programme in the catalogue.";
        assert!(check_spelling_in(text, gb).is_empty(), "{:?}", check_spelling_in(text, gb));
        assert_eq!(suggest_in("licences", 5, us).first().map(String::as_str), Some("licenses"));
        assert_eq!(suggest_in("programmes", 5, us).first().map(String::as_str), Some("programs"));
    }

    /// More British spellings that must stay right in en-GB and en-AU (#1281): -re, single l, ae,
    /// with their inflections.
    #[test]
    fn more_british_spellings_are_right_in_en_gb() {
        let gb = ProofLang::EnGb;
        assert_eq!(ProofLang::from_tag(Some("en-AU")), gb);
        for w in [
            "goitre",
            "goitres",
            "reconnoitre",
            "sceptre",
            "sceptres",
            "enrol",
            "enrols",
            "enrolled",
            "enrolment",
            "fulfil",
            "fulfils",
            "fulfilled",
            "fulfilment",
            "instalment",
            "instalments",
            "skilful",
            "skilfully",
            "wilful",
            "wilfully",
            "encyclopaedia",
            "encyclopaedias",
        ] {
            assert!(is_correct_in(w, gb), "en-GB: {w}");
        }
        let text = "The wilful knight took the sceptre to fulfil his enrolment, paid the instalment and read the encyclopaedia.";
        assert!(check_spelling_in(text, gb).is_empty(), "{:?}", check_spelling_in(text, gb));
    }

    #[test]
    fn user_dictionary_accepts_the_other_dialects_spelling() {
        add_word("savour");
        assert!(is_correct_in("savour", ProofLang::En), "a word the user added is never flagged");
        assert!(!is_correct_in("savor", ProofLang::EnGb));
        remove_word("savour");
        assert!(!is_correct_in("savour", ProofLang::En));
    }

    #[test]
    fn supplement_is_sorted_lowercase() {
        let words: Vec<&str> = EXTRA_EN.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert!(words.len() > 100);
        assert!(words.windows(2).all(|p| p[0] < p[1]), "not sorted / duplicate");
        assert!(words.iter().all(|w| w.chars().all(|c| c.is_lowercase() || c == '\'' || c == '-')), "not lowercase");
    }

    #[test]
    fn spelling_issues_in_text() {
        let t = "This sentense has a mispelled word, see https://example.com/xyzq.";
        let v = check_spelling(t);
        let bad: Vec<&str> = v.iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["sentense", "mispelled"]);
    }

    #[test]
    fn address_token_covers_every_word_split_out_of_it() {
        // #1335: one whitespace-delimited token, however punctuation splits it, has one
        // web/e-mail classification, shared by all its words.
        let t = "zxq@mail.example.com,blorp;qwirpy wugwug";
        let bad: Vec<&str> = check_spelling(t).iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["wugwug"]);
        let opts = Options { ignore_internet: false, ..Options::default() };
        let bad: Vec<&str> = check_spelling_with(t, &opts).iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad.first(), Some(&"zxq"), "{bad:?}");
        assert!(bad.contains(&"qwirpy"), "{bad:?}");
        assert_eq!(bad.last(), Some(&"wugwug"), "{bad:?}");
    }

    #[test]
    fn address_after_a_wide_space_is_an_internet_token() {
        // A web or e-mail address after a no-break space or an ideographic space is skipped,
        // as it is after a plain space.
        let t = "see\u{a0}https://xyzq.example/blorp and\u{3000}www.qwirpy.example or\u{a0}zxq@mail.example wugwug";
        let bad: Vec<&str> = check_spelling(t).iter().map(|i| &t[i.start..i.end]).collect();
        assert_eq!(bad, vec!["wugwug"]);
        let opts = Options { ignore_internet: false, ..Options::default() };
        let bad: Vec<&str> = check_spelling_with(t, &opts).iter().map(|i| &t[i.start..i.end]).collect();
        assert!(["xyzq", "blorp", "qwirpy", "zxq", "wugwug"].iter().all(|w| bad.contains(w)), "{bad:?}");
    }

    #[test]
    fn spelling_scales_over_one_long_token() {
        // #1335: thousands of punctuation-separated words inside a single
        // whitespace-free token: one classification for the token, one issue per
        // misspelled word, through every public spelling entry point.
        let word = |i: usize| {
            let mut w = String::from("zxq");
            let mut n = i;
            loop {
                w.push((b'a' + (n % 26) as u8) as char);
                n /= 26;
                if n == 0 {
                    break;
                }
            }
            w
        };
        let t = (0..2000).map(word).collect::<Vec<_>>().join(",");
        assert_eq!(check_spelling(&t).len(), 2000);
        assert_eq!(check_spelling_in(&t, ProofLang::En).len(), 2000);
        let opts = Options { ignore_internet: false, ..Options::default() };
        assert_eq!(check_spelling_with(&t, &opts).len(), 2000);
        assert_eq!(check_text(&t, &[]).into_iter().filter(|i| i.kind == IssueKind::Spelling).count(), 2000);
    }

    #[test]
    fn grammar_issues() {
        let t = "this is is a apple , and an car.  Done";
        let v = check_grammar(t);
        let msgs: Vec<&str> = v.iter().map(|i| i.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("Repeated")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("\"an\"")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("\"a\" before")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("space before")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Capitalize")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Extra space")), "{msgs:?}");
        assert!(check_grammar("A university is an hour away.").is_empty(), "{:?}", check_grammar("A university is an hour away."));
    }

    #[test]
    fn ellipses_do_not_start_sentences() {
        for text in [
            "I saw... a figure right outside.",
            "I saw… a figure right outside.",
            "I saw.... a figure right outside.",
            "I saw... \"a figure\" right outside.",
            "I saw… “a figure” right outside.",
        ] {
            let capitals: Vec<_> =
                check_grammar(text).into_iter().filter(|issue| issue.message == "Capitalize the first word of a sentence").collect();
            assert!(capitals.is_empty(), "{text}: {capitals:?}");
        }

        for text in ["I saw. a figure.", "I saw! a figure.", "I saw? a figure.", "I saw.\" a figure."] {
            let capitals: Vec<_> =
                check_grammar(text).into_iter().filter(|issue| issue.message == "Capitalize the first word of a sentence").collect();
            assert_eq!(capitals.len(), 1, "{text}: {capitals:?}");
            assert_eq!(capitals[0].suggestions, ["A"], "{text}");
        }
    }

    #[test]
    fn user_dictionary_works() {
        assert!(!is_correct("zxqwordcrafty"));
        add_word("zxqwordcrafty");
        assert!(is_correct("zxqwordcrafty"));
    }

    /// #40: the user dictionary survives a save and a fresh load.
    #[test]
    fn user_dictionary_persists_through_a_file() {
        let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "persist"));
        let _ = std::fs::remove_dir_all(&dir);
        // The folder is created on the first save.
        let path = dir.join("sub/custom-dictionary.txt");
        add_word("zxqpersistone");
        add_word("zxqpersisttwo");
        save_user_dictionary(&path, &[]).expect("save works");
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.lines().any(|w| w == "zxqpersistone") && saved.lines().any(|w| w == "zxqpersisttwo"), "{saved}");
        // Written atomically: no temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(dir.join("sub")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["custom-dictionary.txt"]);
        remove_word("zxqpersistone");
        assert!(!is_correct("zxqpersistone"));
        load_user_dictionary(&path);
        assert!(is_correct("zxqpersistone"), "loading brings the word back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #40: a hostile file is capped: invalid lines are skipped, and only the first megabyte is read.
    #[test]
    fn hostile_dictionary_files_are_capped() {
        let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "hostile"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom-dictionary.txt");
        let mut hostile = vec![0xff, 0xfe, b'\n'];
        hostile.extend(b"zxqhostileok\n  zxqhostiletrim  \nzxq hostile space\nzxq\x07bell\n");
        hostile.extend("zxq".repeat(30).bytes()); // 90 characters: too long
        hostile.push(b'\n');
        // Pad past the 1 MB cap with blank lines; the word after it is never read.
        hostile.resize(1_000_100, b'\n');
        hostile.extend(b"zxqhostilepastcap\n");
        std::fs::write(&path, &hostile).unwrap();
        let added = load_user_dictionary(&path);
        assert!(added.contains(&"zxqhostileok".to_string()) && added.contains(&"zxqhostiletrim".to_string()), "{added:?}");
        assert!(!added.iter().any(|w| w.contains("space") || w.contains("bell") || w.chars().count() > MAX_CUSTOM_WORD_CHARS), "{added:?}");
        assert!(!is_correct("zxqhostilepastcap"), "the file is read only up to the cap");
        assert_eq!(read_dictionary_file(&path).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #40: saving merges in words another process saved, and leaves out the ones just removed.
    #[test]
    fn saving_merges_with_the_file() {
        let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "merge"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom-dictionary.txt");
        std::fs::write(&path, "zxqmergeother\nzxqmergegone\n").unwrap();
        add_word("zxqmergemine");
        save_user_dictionary(&path, &["zxqmergegone".to_string()]).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.lines().any(|w| w == "zxqmergeother"), "the other process's word is kept: {saved}");
        assert!(saved.lines().any(|w| w == "zxqmergemine"), "{saved}");
        assert!(!saved.lines().any(|w| w == "zxqmergegone"), "a removed word stays out: {saved}");
        assert!(is_correct("zxqmergeother") && !is_correct("zxqmergegone"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ignore All lasts for the session: the word is accepted but isn't in the dictionary.
    #[test]
    fn ignored_words_are_not_dictionary_words() {
        assert!(!is_correct("zxqignoredword"));
        ignore_word("ZxqIgnoredWord");
        assert!(is_correct("zxqignoredword"));
        assert!(!user_dictionary().contains(&"zxqignoredword".to_string()));
    }

    #[test]
    fn custom_words_are_single_short_words() {
        assert_eq!(custom_word("  Hello "), Some("hello".into()));
        for bad in ["", "   ", "two words", "tab\tbed", "bell\u{7}", "bad\u{FFFD}", &"x".repeat(MAX_CUSTOM_WORD_CHARS + 1)] {
            assert_eq!(custom_word(bad), None, "{bad:?}");
        }
        assert!(custom_word(&"é".repeat(MAX_CUSTOM_WORD_CHARS)).is_some(), "the cap counts characters, not bytes");
    }

    /// #40: a missing file loads as empty, never an error.
    #[test]
    fn missing_dictionary_file_loads_empty() {
        let path = std::env::temp_dir().join(format!("wc-dict-{}-missing.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(load_user_dictionary(&path).is_empty());
    }

    /// #100: runs are proofed when their language has a dictionary (the default language too)
    /// and skipped otherwise.
    #[test]
    fn dictionary_lang_tags() {
        for l in [None, Some(""), Some("  "), Some("en"), Some("en-US"), Some("en-GB"), Some("EN-us"), Some("en_US")] {
            assert!(has_dictionary(l), "{l:?}");
        }
        for l in [Some("lt-LT"), Some("fr-FR"), Some("de"), Some("uk"), Some("-US"), Some("e")] {
            assert!(!has_dictionary(l), "{l:?}");
        }
        // The gate follows the dictionaries, not English: every proofed language is proofed.
        for d in ProofLang::PROOFED.map(ProofLang::code) {
            assert!(has_dictionary(Some(d)), "{d}");
            assert!(has_dictionary(Some(&format!("{}-XX", d.to_ascii_uppercase()))), "{d}");
        }
    }

    /// Combining marks stay in their word (#3562 review): a decomposed word is one word, with the
    /// byte range of what was typed, and it is checked as its composed form in every language.
    #[test]
    fn decomposed_words_stay_whole() {
        let text = "tambie\u{301}n, cafe\u{301}  \u{301}x";
        let ws: Vec<&str> = words(text).into_iter().map(|(a, b)| &text[a..b]).collect();
        assert_eq!(ws, ["tambie\u{301}n", "cafe\u{301}", "\u{301}x"]);
        assert_eq!(words("tambie\u{301}n"), [(0, 9)]);
        assert_eq!(&*composed("tambie\u{301}n"), "también");
        assert!(matches!(composed("también"), std::borrow::Cow::Borrowed(_)) && matches!(composed("plain"), std::borrow::Cow::Borrowed(_)));
        pt::test_dict::install();
        assert!(is_correct_in("corac\u{327}a\u{303}o", ProofLang::Pt), "Portuguese gets the same composition");
    }

    /// #3562 review: Add to Dictionary, Ignore All, removal and the dictionary file use the same
    /// composed key as spelling, so a word added or ignored decomposed is accepted in both forms,
    /// an NFD entry in a saved file loads (and saves) composed, and either form removes it.
    #[test]
    fn user_dictionary_and_ignore_all_match_both_normalization_forms() {
        let _guard = DICTIONARY_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        es::test_dict::install();
        let es = ProofLang::Es;
        let (nfd, nfc) = ("zqprue\u{301}ba", "zqpruéba");
        let text = format!("esto {nfd}");
        let issues = check_text(&text, &[(0..text.len(), es)]);
        assert_eq!(issues.iter().map(|i| (i.start, i.end)).collect::<Vec<_>>(), [(5, text.len())], "flagged at the typed bytes");
        assert!(!is_correct_in(nfc, es));
        add_word(nfd);
        assert!(is_correct_in(nfd, es) && is_correct_in(nfc, es));
        assert!(check_text(&text, &[(0..text.len(), es)]).is_empty());
        assert!(user_dictionary().contains(&nfc.to_string()) && !user_dictionary().contains(&nfd.to_string()));
        assert!(remove_word(nfc), "the composed form removes the entry added decomposed");
        assert!(!is_correct_in(nfd, es) && !is_correct_in(nfc, es));
        // Ignore All, either way round.
        ignore_word("zqigno\u{301}ro");
        assert!(is_correct_in("zqignóro", es) && is_correct_in("zqigno\u{301}ro", es));
        ignore_word("zqcallé");
        assert!(is_correct_in("zqcalle\u{301}", es));
        // A dictionary file with a decomposed entry loads composed and is saved composed.
        let path = std::env::temp_dir().join(format!("wc-nfd-dictionary-{}.txt", std::process::id()));
        std::fs::write(&path, "zqguarda\u{301}do\nzqotra\n").unwrap();
        let added = load_user_dictionary(&path);
        assert!(added.contains(&"zqguardádo".to_string()) && !added.iter().any(|w| w.contains('\u{301}')), "{added:?}");
        assert!(is_correct_in("zqguardádo", es) && is_correct_in("zqguarda\u{301}do", es));
        save_user_dictionary(&path, &["zqotra".to_string()]).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.lines().any(|l| l == "zqguardádo") && !saved.contains('\u{301}') && !saved.lines().any(|l| l == "zqotra"), "{saved}");
        // Removing the decomposed form takes out the composed entry, and the file follows.
        assert!(remove_word("zqguarda\u{301}do"));
        save_user_dictionary(&path, &["zqguarda\u{301}do".to_string()]).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().lines().any(|l| l == "zqguardádo"));
        assert!(!is_correct_in("zqguardádo", es));
        let _ = std::fs::remove_file(&path);
    }

    /// #100: runs in a language without a dictionary are never flagged; English still is.
    #[test]
    fn non_english_words_are_not_flagged() {
        for w in ["ąžuolas", "élève", "über", "labas", "bonjour"] {
            assert!(is_correct_for(w, Some("lt-LT")), "{w}");
            assert!(is_correct_for(w, Some("fr-FR")), "{w}");
        }
        assert!(!is_correct_for("helo", Some("en-US")), "English typos still flagged");
        assert!(!is_correct_for("helo", None), "default language still proofed");
        assert!(is_correct_for("hello", Some("en-US")));
    }

    /// File › Options › Proofing (#485): each "Ignore" choice turned off makes those words count.
    #[test]
    fn ignore_options_decide_what_is_checked() {
        let t = "SENTENSE helo2 www.exampel.org";
        let bad = |o: Options| check_spelling_with(t, &o).iter().map(|i| t[i.start..i.end].to_string()).collect::<Vec<_>>();
        assert!(bad(Options::default()).is_empty(), "Word's defaults skip all three");
        let all = Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
        assert_eq!(bad(all), ["SENTENSE", "helo2", "www", "exampel"]);
        assert_eq!(bad(Options { ignore_uppercase: false, ..Options::default() }), ["SENTENSE"]);
        assert!(is_correct_with("NASA", &all), "capitals that are a word stay correct");
    }

    #[test]
    fn hyphenation() {
        let l = hyphen::Limits::default();
        assert!(hyphen::hyphenate_word("hyphenation", &l).matches('-').count() >= 2);
    }

    fn flagged(t: &str, v: &[Issue]) -> Vec<String> {
        v.iter().filter(|i| i.kind == IssueKind::Spelling).map(|i| t[i.start..i.end].to_string()).collect()
    }

    #[test]
    fn each_language_uses_its_own_tools() {
        pl::test_dict::install();
        let pl = "Zażółć gęślą jaźń, chrząszcz brzmi w trzcinie.";
        assert!(check_spelling_in(pl, ProofLang::Pl).is_empty(), "{:?}", check_spelling_in(pl, ProofLang::Pl));
        assert!(check_spelling_in(pl, ProofLang::En).len() >= 3, "Polish words are English misspellings");
        assert!(check_spelling_in("Nous écrivons en français.", ProofLang::Other).is_empty());
        assert!(check_grammar_in("this is is wrong", ProofLang::Other).is_empty());
        assert!(is_correct_in("anything", ProofLang::Other) && suggest_in("xyzzy", 5, ProofLang::Other).is_empty());
        let v = check_spelling_in("Płynie żeka.", ProofLang::Pl);
        assert_eq!(flagged("Płynie żeka.", &v), ["żeka"]);
        assert_eq!((v[0].lang, v[0].message.as_str()), (ProofLang::Pl, "Możliwy błąd pisowni"));
        assert!(suggest_in("żeka", 5, ProofLang::Pl).contains(&"rzeka".to_string()));
        assert!(suggest_in("recieve", 5, ProofLang::En).contains(&"receive".to_string()));
        // Polish grammar speaks Polish and has no English a/an rule.
        let g = check_grammar_in("Wiem że to jest a apple.", ProofLang::Pl);
        assert!(g.iter().any(|i| i.message == "Brak przecinka przed „że”"), "{g:?}");
        assert!(!g.iter().any(|i| i.message.contains("\"an\"")), "{g:?}");
    }

    #[test]
    fn italian_uses_its_own_tools() {
        it::test_dict::install();
        let it = "La città vecchia ha un bel caffè in piazza.";
        assert!(check_spelling_in(it, ProofLang::It).is_empty(), "{:?}", check_spelling_in(it, ProofLang::It));
        assert!(check_spelling_in(it, ProofLang::En).len() >= 3, "Italian words are English misspellings");
        let v = check_spelling_in("Vado a Roma con lbiro.", ProofLang::It);
        assert_eq!(flagged("Vado a Roma con lbiro.", &v), ["lbiro"]);
        assert_eq!((v[0].lang, v[0].message.as_str()), (ProofLang::It, "Possibile errore di ortografia"));
        assert!(suggest_in("caffe", 5, ProofLang::It).contains(&"caffè".to_string()));
        // Italian grammar speaks Italian and has no English a/an rule.
        let g = check_grammar_in("questo questo è un test", ProofLang::It);
        assert!(g.iter().any(|i| i.message.contains("ripetuta")), "{g:?}");
    }

    #[test]
    fn mixed_language_paragraphs_check_each_run_in_its_language() {
        pl::test_dict::install();
        // A Polish paragraph quoting English: English spelling and rules inside the quote only.
        let t = "Napisał „the quick brown fox is is here” i poszedł spać, bo żeka wylała.";
        let (a, b) = (t.find("the").unwrap(), t.find('”').unwrap());
        let spans = [(0..a, ProofLang::Pl), (a..b, ProofLang::En), (b..t.len(), ProofLang::Pl)];
        let v = check_text(t, &spans);
        assert_eq!(flagged(t, &v), ["żeka"], "{v:?}");
        let rep = v.iter().find(|i| i.kind == IssueKind::Grammar).unwrap();
        assert_eq!((rep.lang, rep.message.as_str()), (ProofLang::En, "Repeated word: \"is\""), "{v:?}");
        // The English rules don't run over the Polish text (no "Capitalize" for the lower-case
        // Polish words), and the Polish rules don't run over the English quote.
        assert!(v.iter().all(|i| i.lang == ProofLang::Pl || (a..b).contains(&i.start)), "{v:?}");
        // Text outside every span is English; no spans at all is English.
        assert_eq!(flagged("helo świat", &check_text("helo świat", &[])), ["helo", "świat"]);
        assert_eq!(flagged("helo świat", &check_text("helo świat", &[(5..12, ProofLang::Pl)])), ["helo"]);
        // A language without tools: nothing flagged there.
        assert!(check_text("Bonjour mes amis", &[(0..16, ProofLang::Other)]).is_empty());
        // Portuguese checks in their span only, with the Portuguese message.
        pt::test_dict::install();
        let t = "Ele disse que coraçao e frequencia existem.";
        let v = check_text(t, &[(0..t.len(), ProofLang::Pt)]);
        assert_eq!(flagged(t, &v), ["coraçao", "frequencia"], "{v:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::Pt && i.message == "Possível erro de ortografia"), "{v:?}");
        // Italian checks in its span only, with the Italian message.
        it::test_dict::install();
        let t = "Vado a Roma con lbiro e caffe.";
        let v = check_text(t, &[(0..t.len(), ProofLang::It)]);
        assert_eq!(flagged(t, &v), ["lbiro", "caffe"], "{v:?}");
        assert!(v.iter().all(|i| i.lang == ProofLang::It && i.message == "Possibile errore di ortografia"), "{v:?}");
    }

    /// The checks a paragraph gets ([`check_external`] first, else the built-in English ones).
    fn check(text: &str) -> Vec<Issue> {
        check_external(text, "en-US").unwrap_or_else(|| check_text_with(text, &[(0..text.len(), ProofLang::En)], &Options::default(), true, false))
    }

    /// #575: the spelling options apply to an external checker's spelling issues too.
    #[test]
    fn spelling_options_apply_to_external_issues() {
        let text = "NASA b2b see https://exmple.org now";
        let on = Options::default();
        let off = Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
        for (a, b) in [(0, 4), (5, 8), (13, 19)] {
            assert!(spelling_ignored_by_options(text, a, b, &on), "{a}..{b}");
            assert!(!spelling_ignored_by_options(text, a, b, &off), "{a}..{b}");
        }
        assert!(!spelling_ignored_by_options(text, 9, 12, &on));
        assert!(!spelling_ignored_by_options(text, 3, 99, &on));
    }

    /// #575: an external checker's result replaces the built-in checks for exactly the text it
    /// was computed for; edited text falls back; bad ranges are dropped; words the user added
    /// to the dictionary lose their spelling issue.
    #[test]
    fn external_results_apply_to_their_exact_text() {
        let text = "Zqxv external checker teh text 575.";
        assert!(!external::has(text, "en-US"));
        assert_eq!(external::stamp(text, "en-US"), 0);
        let issue = |start, end, kind| Issue { start, end, kind, message: "m".into(), suggestions: vec!["x".into()], lang: ProofLang::En };
        external::put(
            text,
            "en-US",
            vec![
                issue(26, 30, IssueKind::Grammar),
                issue(22, 25, IssueKind::Spelling),
                issue(0, 4, IssueKind::Spelling),
                issue(5, 5, IssueKind::Grammar),    // empty
                issue(30, 999, IssueKind::Grammar), // past the end
                issue(40, 10, IssueKind::Spelling), // reversed
            ],
        );
        let s1 = external::stamp(text, "en-US");
        assert!(s1 > 0);
        let got: Vec<(usize, usize)> = check(text).iter().map(|i| (i.start, i.end)).collect();
        assert_eq!(got, [(0, 4), (22, 25), (26, 30)]);
        // Edited text: the built-in checks again (they flag "Zqxv").
        let edited = "Zqxv external checker teh text 575!";
        assert!(external::get(edited, "en-US").is_none());
        assert!(check(edited).iter().any(|i| i.kind == IssueKind::Spelling && edited.get(i.start..i.end) == Some("Zqxv")));
        // Add to Dictionary hides the external spelling issue for that word.
        add_word("zqxv");
        let got: Vec<(usize, usize)> = check(text).iter().map(|i| (i.start, i.end)).collect();
        assert_eq!(got, [(22, 25), (26, 30)]);
        // A new result gets a new stamp.
        external::put(text, "en-US", Vec::new());
        assert!(external::stamp(text, "en-US") > s1);
        assert!(check(text).is_empty());
    }

    /// #575 review: a result counts only for the language it was checked in. When the paragraph's
    /// language changes, the text is checked again and kept beside the old one.
    #[test]
    fn external_results_belong_to_their_language() {
        let text = "Les enfant 575 jouent.";
        let issue = |start, end| Issue { start, end, kind: IssueKind::Spelling, message: "m".into(), suggestions: Vec::new(), lang: ProofLang::En };
        external::put(text, "en-US", vec![issue(4, 10)]);
        assert!(external::has(text, "en-US"));
        assert!(!external::has(text, "fr-FR"), "a French paragraph needs a French result");
        let s1 = external::stamp(text, "en-US");
        external::put(text, "fr-FR", vec![issue(0, 10)]);
        // Both languages keep their own result, so two paragraphs with the same text in
        // different languages never replace each other.
        assert!(external::has(text, "fr-FR") && external::has(text, "en-US"));
        assert_eq!(external::stamp(text, "en-US"), s1);
        assert_eq!(external::get(text, "fr-FR").map(|v| v.len()), Some(1));
        assert_eq!(check(text).iter().map(|i| (i.start, i.end)).collect::<Vec<_>>(), [(4, 10)]);
    }

    /// #575: a range that cuts a character in two is dropped, never sliced.
    #[test]
    fn external_ranges_off_char_boundaries_are_dropped() {
        let text = "é😀 575 boundaries";
        external::put(
            text,
            "en-US",
            vec![Issue { start: 1, end: 3, kind: IssueKind::Grammar, message: String::new(), suggestions: Vec::new(), lang: ProofLang::En }],
        );
        assert_eq!(external::get(text, "en-US"), Some(Vec::new()));
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,80}") {
            let _ = check(&s);
            let _ = check_spelling(&s);
            let _ = check_grammar(&s);
            let _ = suggest(&s, 3);
            let _ = has_dictionary(Some(&s));
            let _ = is_correct_for(&s, Some(&s));
            let _ = hyphen::hyphen_points(&s, &hyphen::Limits::default());
            for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::Pl, ProofLang::Pt, ProofLang::It, ProofLang::Hr, ProofLang::Tr, ProofLang::Ru, ProofLang::Cs, ProofLang::Id, ProofLang::De, ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy, ProofLang::Other] {
                let _ = check_spelling_in(&s, lang);
                let _ = check_grammar_in(&s, lang);
                let _ = is_correct_in(&s, lang);
                let _ = hyphen::hyphen_points_in(&s, &hyphen::Limits::default(), lang);
            }
            let _ = lang::normalize_tag(&s);
            let _ = ProofLang::from_tag(Some(&s));
        }

        #[test]
        fn polish_suggestions_never_panic(s in "[a-ząćęłńóśźżA-ZĄĆĘŁŃÓŚŹŻ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Pl);
        }

        #[test]
        fn portuguese_suggestions_never_panic(s in "[a-záàâãéêíóôõúüçA-ZÁÀÂÃÉÊÍÓÔÕÚÜÇ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Pt);
        }

        #[test]
        fn italian_suggestions_never_panic(s in "[a-zàèéìòóùA-ZÀÈÉÌÒÓÙ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::It);
        }

        #[test]
        fn turkish_suggestions_never_panic(s in "[a-zçğıöşüA-ZÇĞİÖŞÜ'’ -]{0,14}") {
            let _ = suggest_in(&s, 4, ProofLang::Tr);
        }

        #[test]
        fn czech_suggestions_never_panic(s in "[a-zěščřžýáíéůúĚŠČŘŽÝÁÍÉŮÚ'’ -]{0,14}") {
            cs::test_dict::install();
            let _ = suggest_in(&s, 4, ProofLang::Cs);
        }

        #[test]
        fn indonesian_suggestions_never_panic(s in "[a-zA-Zé'’ -]{0,14}") {
            id::test_dict::install();
            let _ = suggest_in(&s, 4, ProofLang::Id);
            let _ = is_correct_in(&s, ProofLang::Id);
        }

        #[test]
        fn spanish_suggestions_never_panic(s in "[a-zA-ZáéíóúüñÁÉÍÓÚÜÑ¿¡'’ -]{0,14}") {
            es::test_dict::install();
            for lang in [ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy] {
                let _ = suggest_in(&s, 4, lang);
                let _ = is_correct_in(&s, lang);
            }
        }

        #[test]
        fn mixed_spans_never_panic(s in "\\PC{0,60}", cuts in proptest::collection::vec((0usize..80, 0usize..80, 0u8..15), 0..6)) {
            let langs = [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::Pl, ProofLang::Pt, ProofLang::It, ProofLang::Tr, ProofLang::Ru, ProofLang::Cs, ProofLang::Id, ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy, ProofLang::Other];
            let spans: Vec<(Range<usize>, ProofLang)> = cuts.iter().map(|&(a, b, l)| (a.min(b)..a.max(b), langs[l as usize % langs.len()])).collect();
            for i in check_text(&s, &spans) {
                proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
            }
        }
    }

    proptest::proptest! {
        // Suggestions scan the whole word list: fewer cases keep the test quick.
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]
        #[test]
        fn english_dialect_suggestions_never_panic(s in "[a-zA-Zéß'’ -]{0,16}|(colou?r|cent(er|re)|organi[sz]ation)['’s-]{0,4}") {
            for lang in [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa] {
                let _ = suggest_in(&s, 4, lang);
                let _ = is_correct_in(&s, lang);
                let _ = en_variants::preferred(&s, lang);
                let _ = en_variants::known_variant(&s, lang, |_| true);
            }
        }
    }
}
