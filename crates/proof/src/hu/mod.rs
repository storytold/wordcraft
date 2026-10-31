//! Hungarian proofing using the installed Hunspell `hu_HU` dictionary (Magyar Ispell, never
//! bundled). Hungarian is agglutinative and compounds freely (`almafa`, `házaimban`,
//! `újrahasznosíthatóság`), and its dictionary relies on what [`crate::affix`] leaves out:
//! flag aliases (`AF`, `AM`), compounding (`COMPOUNDFLAG`, `COMPOUNDRULE`, `ONLYINCOMPOUND`…),
//! `NEEDAFFIX`, `CIRCUMFIX` and `ICONV`. So the dictionary is read by `spellbook`, a pure-Rust
//! Hunspell-compatible checker (MPL-2.0), which also makes the suggestions. Generic grammar
//! checks have Hungarian messages; hyphenation keeps the fallback, without Hungarian patterns.

pub mod grammar;
#[cfg(test)]
pub(crate) mod test_dict;

use std::path::Path;
use std::sync::{Arc, RwLock};

/// A Hungarian dictionary, or none (Hungarian spelling is off then).
#[derive(Default)]
pub struct HuDict(Option<spellbook::Dictionary>);

impl HuDict {
    /// Parse a Hunspell `.aff`/`.dic` pair (UTF-8 text).
    pub fn from_hunspell(aff: &str, dic: &str) -> Result<HuDict, String> {
        spellbook::Dictionary::new(aff, dic).map(|d| HuDict(Some(d))).map_err(|e| format!("Hungarian dictionary: {e}"))
    }

    /// No dictionary: every word is accepted.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Is `word` (one word, no hyphens split off) in the dictionary, as Hunspell would say?
    pub fn check(&self, word: &str) -> bool {
        self.0.as_ref().is_none_or(|d| d.check(word))
    }

    /// Up to `max` corrections for `word`, best first.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let Some(d) = &self.0 else { return Vec::new() };
        let mut out = Vec::new();
        d.suggest(word, &mut out);
        out.retain(|s| s != word);
        out.truncate(max);
        out
    }
}

/// The Hungarian dictionary: `None` until first use, then the installed one (or an empty one).
static DICT: RwLock<Option<Arc<HuDict>>> = RwLock::new(None);

/// File names of a Hungarian Hunspell dictionary.
const NAMES: &[&str] = &["hu_HU", "hu-HU", "hu"];

/// Longest word (in bytes) checked or corrected; longer input is accepted as is.
const MAX_WORD_BYTES: usize = 120;

/// The Hungarian dictionary: the installed Hunspell `hu_HU` (read on first use), one given
/// with [`install_dictionary`], or an empty one when none is available (see [`has_dictionary`]).
pub fn dictionary() -> Arc<HuDict> {
    if let Some(d) = DICT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return d.clone();
    }
    let loaded = Arc::new(load_installed());
    let mut slot = DICT.write().unwrap_or_else(|e| e.into_inner());
    // Another thread (or an install) may have filled it meanwhile: that one wins.
    slot.get_or_insert(loaded).clone()
}

fn load_installed() -> HuDict {
    let Some((aff, dic)) = crate::hunspell::find(NAMES) else {
        crate::hunspell::clear_load_error(NAMES[0]);
        log::info!("Hungarian spelling: no Hunspell hu_HU dictionary installed");
        return HuDict::default();
    };
    crate::hunspell::note_load(NAMES[0], &dic, load(&aff, &dic)).unwrap_or_else(|e| {
        log::warn!("Hungarian spelling: {e}");
        HuDict::default()
    })
}

/// Read a Hunspell pair from disk, decoded per the `.aff`'s `SET` line. A UTF-8 `.aff` that
/// isn't valid UTF-8 is read with [`byte_flags_to_long`]: that is Magyar Ispell's.
pub fn load(aff: &Path, dic: &Path) -> Result<HuDict, String> {
    let (aff_bytes, dic_bytes) = (crate::hunspell::read_capped(aff)?, crate::hunspell::read_capped(dic)?);
    let (aff_text, dic_text) = match crate::hunspell::decode(&aff_bytes, &dic_bytes) {
        Ok(pair) => pair,
        Err(e) => {
            let dic_text = String::from_utf8(dic_bytes).map_err(|_| e.clone())?;
            (byte_flags_to_long(&aff_bytes).ok_or(e)?, dic_text)
        }
    };
    HuDict::from_hunspell(&aff_text, &dic_text)
}

/// Directives whose one argument is a flag.
const FLAG_DIRECTIVES: &[&[u8]] = &[
    b"CIRCUMFIX",
    b"COMPOUNDBEGIN",
    b"COMPOUNDEND",
    b"COMPOUNDFIRST",
    b"COMPOUNDFLAG",
    b"COMPOUNDFORBIDFLAG",
    b"COMPOUNDLAST",
    b"COMPOUNDMIDDLE",
    b"COMPOUNDPERMITFLAG",
    b"COMPOUNDROOT",
    b"FORBIDDENWORD",
    b"FORCEUCASE",
    b"GENERATE",
    b"KEEPCASE",
    b"LEMMA_PRESENT",
    b"NEEDAFFIX",
    b"NOSUGGEST",
    b"ONLYINCOMPOUND",
    b"ONLYROOT",
    b"PSEUDOROOT",
    b"SUBSTANDARD",
    b"WARN",
];

/// Rewrite a UTF-8 `.aff` with one-byte flags (the default `FLAG` type) as `FLAG long`.
///
/// Magyar Ispell says `SET UTF-8` for its words but writes its ~250 flags as raw bytes, most of
/// them above 0x7F (`SFX \xff Y 3`, `AF V\xcb\xafj…`), as Hunspell's byte flags allow. Such a
/// file isn't valid UTF-8 text, and decoding it lossily would merge distinct flags. So every
/// flag field — `AF` lines, the flag of `PFX`/`SFX` lines, flag directives and `COMPOUNDRULE`
/// patterns — becomes the byte in two hex digits (`\xff` → `ff`, a two-character flag), and
/// everything else is read as UTF-8 (the header comments, in ISO 8859-2, lossily: they carry no
/// rules). With `AF` the `.dic` and the continuation classes already use alias numbers, so they
/// need no change. (`FLAG num` would do too, but `spellbook` doesn't look numeric flags up as
/// `AF` aliases.) `None` when the file names a `FLAG` type of its own (nothing to rewrite), or
/// has no `AF` table: its `.dic` would then carry raw byte flags that no longer match the rules.
pub fn byte_flags_to_long(aff: &[u8]) -> Option<String> {
    let num = |b: u8| format!("{b:02x}");
    let mut out = String::with_capacity(aff.len() + aff.len() / 8);
    out.push_str("FLAG long\n");
    // `AF` and `COMPOUNDRULE` tables open with a count line, which stays as it is.
    let (mut af_count_seen, mut rule_count_seen) = (false, false);
    let mut aliases = false;
    for line in aff.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let (key, after_key) = split_field(line);
        let (arg, rest) = split_field(after_key);
        let rest = String::from_utf8_lossy(rest);
        let rewritten = match key {
            b"FLAG" => return None,
            b"AF" if !std::mem::replace(&mut af_count_seen, true) => None,
            // An alias line (`AF V\xcb\xafj… # 1`).
            b"AF" if !arg.is_empty() => {
                aliases = true;
                Some(format!("AF {} {rest}", arg.iter().map(|&b| num(b)).collect::<String>()))
            }
            b"PFX" | b"SFX" if arg.len() == 1 => {
                Some(format!("{} {} {rest}", String::from_utf8_lossy(key), arg.first().copied().map(num).unwrap_or_default()))
            }
            b"COMPOUNDRULE" if !std::mem::replace(&mut rule_count_seen, true) => None,
            // `0*1` → `(48)*(49)`.
            b"COMPOUNDRULE" => Some(format!(
                "COMPOUNDRULE {}",
                arg.iter().map(|&b| if b == b'*' || b == b'?' { char::from(b).to_string() } else { format!("({})", num(b)) }).collect::<String>()
            )),
            k if arg.len() == 1 && FLAG_DIRECTIVES.contains(&k) => {
                Some(format!("{} {}", String::from_utf8_lossy(k), arg.first().copied().map(num).unwrap_or_default()))
            }
            _ => None,
        };
        match rewritten {
            Some(l) => out.push_str(l.trim_end()),
            None => out.push_str(&String::from_utf8_lossy(line)),
        }
        out.push('\n');
    }
    aliases.then_some(out)
}

/// The first space- or tab-separated field of `line` and what follows it (leading blanks
/// skipped on both).
fn split_field(line: &[u8]) -> (&[u8], &[u8]) {
    let blank = |b: &u8| *b == b' ' || *b == b'\t';
    let start = line.iter().position(|b| !blank(b)).unwrap_or(line.len());
    let line = line.get(start..).unwrap_or_default();
    let end = line.iter().position(blank).unwrap_or(line.len());
    let (field, rest) = line.split_at(end);
    let skip = rest.iter().position(|b| !blank(b)).unwrap_or(rest.len());
    (field, rest.get(skip..).unwrap_or_default())
}

/// Where the installed Hungarian `.dic` is (`None`: not installed).
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::find(NAMES).map(|(_, dic)| dic)
}

/// Use this Hungarian dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to
/// UTF-8), for example one the user picked. On error the current dictionary stays.
pub fn install_dictionary(aff: &str, dic: &str) -> Result<(), String> {
    let d = HuDict::from_hunspell(aff, dic)?;
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(d));
    Ok(())
}

/// Forget the loaded dictionary, so the next check looks for an installed one again (after
/// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
pub fn reload_dictionary() {
    *DICT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Is a Hungarian dictionary available (spelling is checked)?
pub fn has_dictionary() -> bool {
    !dictionary().is_empty()
}

/// Is `c` a letter of the Latin script (the only one the Hungarian dictionary covers)?
fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c < '\u{0250}' || ('\u{1E00}'..='\u{1EFF}').contains(&c))
}

/// The word without surrounding punctuation and apostrophes.
fn core(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’' && c != '-').trim_matches(['\'', '’', '-'])
}

/// Is `word` spelled correctly in Hungarian? Numbers, single letters, short words in capitals
/// (acronyms: `MÁV`, `EU`), words with digits (`2026-ban`) and words in other scripts are
/// accepted, and so is every word when no Hungarian dictionary is installed.
pub fn is_correct(word: &str) -> bool {
    is_correct_with(&dictionary(), word)
}

/// [`is_correct`] against a given dictionary (an empty one accepts everything).
pub fn is_correct_with(d: &HuDict, word: &str) -> bool {
    if d.is_empty() {
        return true; // no Hungarian dictionary installed: no spelling marks
    }
    let w = core(word);
    let n = w.chars().count();
    if n <= 1 || w.len() > MAX_WORD_BYTES || w.chars().any(|c| c.is_ascii_digit()) || !w.chars().any(char::is_alphabetic) {
        return true;
    }
    if w.chars().all(|c| !c.is_lowercase()) && n <= 6 {
        return true;
    }
    if !w.chars().any(is_latin_letter) {
        return true; // other scripts: no dictionary
    }
    // The dictionary knows hyphenated compounds (`helyesírás-ellenőrző`); else each part.
    known(d, w) || (w.contains('-') && w.split('-').all(|p| p.chars().count() <= 1 || known(d, p)))
}

/// Is one word in the dictionary or the user dictionary?
fn known(d: &HuDict, w: &str) -> bool {
    d.check(w) || crate::in_user_dictionary(&w.to_lowercase())
}

/// Corrections for a misspelled Hungarian word, best first (none without a dictionary).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let w = core(word);
    if w.is_empty() || w.len() > MAX_WORD_BYTES {
        return Vec::new();
    }
    dictionary().suggest(w, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_lookup_accepts_hungarian_file_names() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hu-aliases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["hu_HU", "hu-HU", "hu"] {
            std::fs::write(dir.join(format!("{name}.aff")), test_dict::AFF).unwrap();
            std::fs::write(dir.join(format!("{name}.dic")), test_dict::DIC).unwrap();
            let (aff, dic) = crate::hunspell::find_in(&dir, NAMES).unwrap();
            let d = load(&aff, &dic).unwrap();
            assert!(is_correct_with(&d, "házban"));
            std::fs::remove_file(aff).unwrap();
            std::fs::remove_file(dic).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn byte_flags_without_aliases_are_not_rewritten() {
        // Without `AF` the `.dic` would keep raw byte flags (`/\xe1`) that no rewritten rule matches.
        assert_eq!(byte_flags_to_long(b"SET UTF-8\nSFX \xe1 Y 1\nSFX \xe1 0 ban .\n"), None);
        assert!(byte_flags_to_long(b"SET UTF-8\nAF 1\nAF \xe1\nSFX \xe1 Y 1\nSFX \xe1 0 ban .\n").is_some());
    }

    #[test]
    fn latin2_comments_in_a_utf8_affix_file_are_skipped() {
        // Magyar Ispell's header names its authors in ISO 8859-2 (`L\xe1szl\xf3`).
        let dir = std::env::temp_dir().join(format!("wordcraft-hu-latin2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Its flags are raw bytes too: here suffix class `A` is `\xe1` and `B` is `\xcb`.
        let mut aff = b"# N\xe9meth L\xe1szl\xf3\n".to_vec();
        let flags =
            test_dict::AFF.replace("AF AY", "AF \u{1}Y").replace("AF BY", "AF \u{2}Y").replace("SFX A", "SFX \u{1}").replace("SFX B", "SFX \u{2}");
        aff.extend(flags.bytes().map(|b| match b {
            1 => 0xe1,
            2 => 0xcb,
            b => b,
        }));
        assert!(String::from_utf8(aff.clone()).is_err());
        std::fs::write(dir.join("hu_HU.aff"), aff).unwrap();
        std::fs::write(dir.join("hu_HU.dic"), test_dict::DIC).unwrap();
        let d = load(&dir.join("hu_HU.aff"), &dir.join("hu_HU.dic")).unwrap();
        for w in ["almafa", "házban", "kertek", "házkertben"] {
            assert!(is_correct_with(&d, w), "{w}");
        }
        for w in ["hazban", "kertban", "házben"] {
            assert!(!is_correct_with(&d, w), "{w}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hungarian_suffixes_compounds_aliases_case_and_diacritics() {
        test_dict::install();
        assert!(has_dictionary());
        for w in [
            "ház",
            "házban",
            "házak",
            "kertben",
            "kertek",
            "almafa",
            "házkert",
            "házkertben",
            "Ház",
            "HÁZBAN",
            "helyesírás",
            "helyesírás-ellenőrző",
            "ellenőrző",
            "„házban”",
            "2026-ban",
            "MÁV",
        ] {
            assert!(is_correct(w), "{w}");
        }
        for w in ["hazban", "kertban", "házben", "almafak", "helyesirás", "helyesírás-ellenörző", "hÁz"] {
            assert!(!is_correct(w), "{w}");
        }
        for c in "áéíóöőúüű".chars() {
            assert!(test_dict::AFF.contains(c) || test_dict::DIC.contains(c), "UTF-8 fixture covers {c}");
        }
    }

    #[test]
    fn forbidden_words_are_rejected_in_every_case() {
        test_dict::install();
        for w in ["zqtiltott", "Zqtiltott", "ZQTILTOTT"] {
            assert!(!is_correct(w), "{w}");
        }
    }

    #[test]
    fn suggestions_restore_accents_and_vowel_harmony() {
        test_dict::install();
        assert!(suggest("hazban", 8).contains(&"házban".into()), "{:?}", suggest("hazban", 8));
        assert!(suggest("helyesirás", 8).contains(&"helyesírás".into()));
        assert!(suggest("kertban", 8).contains(&"kertben".into()), "{:?}", suggest("kertban", 8));
        assert!(suggest("hazban", 1).len() <= 1);
        assert!(suggest("", 8).is_empty() && suggest(&"a".repeat(200), 8).is_empty());
    }

    #[test]
    fn missing_dictionary_and_ignored_inputs_are_quiet() {
        let empty = HuDict::default();
        assert!(is_correct_with(&empty, "hazban") && empty.suggest("hazban", 4).is_empty());
        let d = HuDict::from_hunspell(test_dict::AFF, test_dict::DIC).unwrap();
        for w in ["", "a", "2026", "EU", "cikk2", "日本語", &"x".repeat(200)] {
            assert!(is_correct_with(&d, w), "{w}");
        }
    }

    #[test]
    fn user_dictionary_applies() {
        test_dict::install();
        assert!(!is_correct("zqsajátmagyarszó"));
        crate::add_word("Zqsajátmagyarszó");
        assert!(is_correct("zqsajátmagyarszó"));
    }

    #[test]
    fn hungarian_dispatch_messages_options_and_mixed_spans() {
        test_dict::install();
        assert!(crate::has_dictionary(Some("hu-HU")));
        assert!(crate::is_correct_for("házban", Some("hun")));
        let text = "Házban hazban";
        let issues = crate::check_text(text, &[(0..text.len(), crate::ProofLang::Hu)]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(&text[issues[0].start..issues[0].end], "hazban");
        assert_eq!(issues[0].message, "Lehetséges helyesírási hiba");
        assert_eq!(issues[0].lang, crate::ProofLang::Hu);
        assert!(crate::suggest_in("hazban", 8, crate::ProofLang::Hu).contains(&"házban".into()));
        let text = "Ház this is is wrong";
        let issues = crate::check_text(text, &[(0..4, crate::ProofLang::Hu), (5..text.len(), crate::ProofLang::En)]);
        assert!(issues.iter().any(|i| i.lang == crate::ProofLang::En && i.message.starts_with("Repeated word")));
        assert!(issues.iter().all(|i| i.start >= 5));
        let opts = crate::Options { ignore_uppercase: false, ..Default::default() };
        assert!(!crate::is_correct_in_with("ZQTILTOTT", crate::ProofLang::Hu, &opts));
    }

    /// The real Magyar Ispell dictionary, when `WORDCRAFT_HU_DICT_DIR` names a folder with
    /// `hu_HU.aff`/`hu_HU.dic` (skipped otherwise: the dictionary is GPL/LGPL/MPL and not in
    /// the repository).
    #[test]
    fn real_magyar_ispell_when_available() {
        let Some(dir) = std::env::var_os("WORDCRAFT_HU_DICT_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        // The first Hungarian check pays for this load (reported, not asserted: CI machines vary).
        let start = std::time::Instant::now();
        let d = load(&dir.join("hu_HU.aff"), &dir.join("hu_HU.dic")).unwrap();
        eprintln!("hu_HU: loaded in {:?}", start.elapsed());
        for w in ["házaimban", "almafákon", "legeslegszebb", "Budapesten", "helyesírás-ellenőrző", "fiókjaitokból", "újrahasznosíthatóság"]
        {
            assert!(is_correct_with(&d, w), "{w}");
        }
        for w in ["házaimbam", "helyesirás", "Magyarorszag", "almafakon"] {
            assert!(!is_correct_with(&d, w), "{w}");
        }
        let start = std::time::Instant::now();
        assert!(d.suggest("helyesirás", 4).contains(&"helyesírás".into()));
        for w in ["házaimbam", "szamitogep", "kutyáinkk", "asdfghjkl"] {
            let _ = d.suggest(w, 8);
        }
        eprintln!("hu_HU: 5 suggestion lists in {:?}", start.elapsed());
    }
}
