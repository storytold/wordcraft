//! Vietnamese: telling Vietnamese text apart from English, so a document typed in Vietnamese under
//! the default English proofing language is spelled as Vietnamese (as Word's "detect language
//! automatically" does), and spelling it.
//!
//! Spelling is by syllable, not by word: every Vietnamese word is written as syllables, and a
//! syllable is an onset (`ngh`, `tr`, `qu`…), a rhyme from the closed set of Vietnamese rhymes
//! (`ương`, `oanh`, `uyêt`…) and at most one tone mark. The spelling rules on top: `k`, `gh` and
//! `ngh` only before `i e ê y`, `c`, `g` and `ng` never before them, and a rhyme ending in a stop
//! (`c ch p t`) takes only the acute or the dot below. That catches the common slips (`nghành`,
//! `ngỉ`, `hoc`, leftover Telex keys like `tieengs`). The rules still allow syllables nobody writes
//! (`quyêm`), so when a list of syllables in use is installed ([`SYLLABLES_FILE`] in a
//! dictionaries folder, like a Hunspell dictionary) a syllable must also be on it. WordCraft doesn't
//! ship one: a list extracted from the Vietnamese Wiktionary is CC BY-SA, so it stays with the user.
//!
//! The test is orthographic. Vietnamese writes most syllables with letters no other Latin-script
//! language shares: the horn letters (`ơ ư`), `ĩ ũ`, and the tone-marked vowels of the Latin
//! Extended Additional block (`ạ ả ấ ầ … ỹ`), precomposed or as combining hook above, dot below
//! and horn. French and Portuguese share `â ê ô à é`, Romanian `ă` and Croatian, Serbian and
//! Bosnian `đ`, so those alone don't count.

use std::sync::{Arc, RwLock};

/// Does `c` occur in Vietnamese words but (practically) nowhere else?
fn vietnamese_letter(c: char) -> bool {
    matches!(c, 'ơ' | 'Ơ' | 'ư' | 'Ư' | 'ĩ' | 'Ĩ' | 'ũ' | 'Ũ' | '\u{1EA0}'..='\u{1EF9}' | '\u{0309}' | '\u{0323}' | '\u{031B}')
}

/// Is `word` written with a Vietnamese-only letter (`Việt`, `ơn`)? Such a word is never an
/// English misspelling.
pub fn is_vietnamese_word(word: &str) -> bool {
    let combining = |c: char| matches!(c, '\u{0300}'..='\u{036F}');
    // Two marks stacked on one letter (decomposed `ế` = e + circumflex + acute) are Vietnamese too.
    word.chars().any(vietnamese_letter) || word.chars().zip(word.chars().skip(1)).any(|(a, b)| combining(a) && combining(b))
}

/// Does a paragraph read as Vietnamese? At least two of its words (any, in a heading of up to three
/// words) carry a Vietnamese-only letter, and they are at least a sixth of its words: Vietnamese syllables without
/// such a letter (`anh`, `con`, `ba`) are common, while an English paragraph quoting one
/// Vietnamese name stays English.
pub fn looks_vietnamese(text: &str) -> bool {
    let mut total = 0usize;
    let mut marked = 0usize;
    for w in text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '\u{0300}'..='\u{036F}'))).filter(|w| !w.is_empty()) {
        total += 1;
        if is_vietnamese_word(w) {
            marked += 1;
        }
    }
    marked > 0 && (marked >= 2 || total <= 3) && marked * 6 >= total
}

/// Each vowel with its tones, in tone order: level, grave, acute, hook above, tilde, dot below.
const VOWELS: [&str; 12] = ["aàáảãạ", "ăằắẳẵặ", "âầấẩẫậ", "eèéẻẽẹ", "êềếểễệ", "iìíỉĩị", "oòóỏõọ", "ôồốổỗộ", "ơờớởỡợ", "uùúủũụ", "ưừứửữự", "yỳýỷỹỵ"];

const ONSETS: [&str; 27] = [
    "ngh", "ng", "nh", "ch", "gh", "gi", "kh", "ph", "th", "tr", "qu", "b", "c", "d", "đ", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "x",
];

/// Every Vietnamese rhyme, without its tone.
const RHYMES: &[&str] = &[
    "a", "ac", "ach", "ai", "am", "an", "ang", "anh", "ao", "ap", "at", "au", "ay", "ăc", "ăm", "ăn", "ăng", "ăp", "ăt", "âc", "âm", "ân", "âng",
    "âp", "ât", "âu", "ây", "e", "ec", "em", "en", "eng", "eo", "ep", "et", "ê", "êch", "êm", "ên", "ênh", "êp", "êt", "êu", "i", "ia", "ich", "im",
    "in", "inh", "ip", "it", "iu", "iêc", "iêm", "iên", "iêng", "iêp", "iêt", "iêu", "y", "yêm", "yên", "yêt", "yêu", "o", "oc", "oi", "om", "on",
    "ong", "op", "ot", "ooc", "oong", "ô", "ôc", "ôi", "ôm", "ôn", "ông", "ôp", "ôt", "ơ", "ơi", "ơm", "ơn", "ơp", "ơt", "u", "ua", "uc", "ui", "um",
    "un", "ung", "up", "ut", "uôc", "uôi", "uôm", "uôn", "uông", "uôt", "ư", "ưa", "ưc", "ưi", "ưm", "ưng", "ưt", "ưu", "ươc", "ươi", "ươm", "ươn",
    "ương", "ươp", "ươt", "ươu", "oa", "oac", "oach", "oai", "oam", "oan", "oang", "oanh", "oao", "oap", "oat", "oay", "oăc", "oăm", "oăn", "oăng",
    "oăt", "oe", "oen", "oeo", "oet", "uân", "uâng", "uât", "uây", "uê", "uêch", "uênh", "uơ", "uy", "uych", "uynh", "uyp", "uyt", "uya", "uyên",
    "uyêt", "uyu",
];

/// Is `word` spelled correctly in Vietnamese? Each of its syllables (a hyphen or apostrophe
/// separates them) must be a Vietnamese syllable; a plain-ASCII word may also be an English one
/// (`email`, `file` are everyday in Vietnamese text).
pub fn is_correct(word: &str) -> bool {
    is_correct_with(word, &known())
}

/// [`is_correct`] against a given list of syllables in use (an empty one: the rules alone).
fn is_correct_with(word: &str, known: &Syllables) -> bool {
    let lower = word.to_lowercase();
    if crate::in_user_dictionary(&lower) {
        return true;
    }
    lower.split(['-', '\'', '’']).filter(|s| !s.is_empty()).all(|s| syllable(s, known)) || (word.is_ascii() && crate::is_correct(word))
}

/// The file name of a list of Vietnamese syllables in use, one per line (`#` starts a comment
/// line), looked for in the same folders as Hunspell dictionaries.
pub const SYLLABLES_FILE: &str = "vi-syllables.txt";

/// Largest list read (Vietnamese writes fewer than 10,000 syllables, about 60 KB).
const MAX_FILE: u64 = 4 << 20;

/// Syllables keyed by toneless letters and tone, so decomposed input matches too.
type Syllables = std::collections::HashSet<(String, usize)>;

/// The installed list, read on first use; `None` until then or after [`reload_dictionary`].
static KNOWN: RwLock<Option<Arc<Syllables>>> = RwLock::new(None);

fn known() -> Arc<Syllables> {
    if let Some(k) = KNOWN.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return k;
    }
    let k = Arc::new(installed_path().and_then(|p| load(&p)).unwrap_or_default());
    *KNOWN.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&k));
    k
}

/// Read a list from disk (size-capped); `None` when it can't be read.
fn load(path: &std::path::Path) -> Option<Syllables> {
    let len = std::fs::metadata(path).ok()?.len();
    (len <= MAX_FILE).then(|| std::fs::read_to_string(path).ok()).flatten().map(|t| parse_list(&t))
}

/// The syllables of a list's text that keep the rules (others can never match a word anyway).
fn parse_list(text: &str) -> Syllables {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| parse(&l.to_lowercase()))
        .filter(|(s, t)| valid(s, *t))
        .collect()
}

/// The installed list of syllables in use, searching the dictionaries folder and the system
/// locations now; `None` when there is none (the rules alone check spelling).
pub fn installed_path() -> Option<std::path::PathBuf> {
    crate::hunspell::search_dirs(None).into_iter().map(|d| d.join(SYLLABLES_FILE)).find(|p| p.is_file())
}

/// Forget the loaded list, so the next check looks for an installed one again.
pub fn reload_dictionary() {
    *KNOWN.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Is `s` (lower case) one Vietnamese syllable? The rules alone accept syllables nobody writes
/// (`quyêm`), so with a list installed it must also be on it.
fn syllable(s: &str, known: &Syllables) -> bool {
    parse(s).is_some_and(|k| valid(&k.0, k.1) && (known.is_empty() || known.contains(&k)))
}

/// `s` (lower case) as its toneless letters and tone (0 none, 1 grave, 2 acute, 3 hook, 4 tilde,
/// 5 dot below), or `None` if it isn't written with Vietnamese letters and at most one tone. Tones
/// and vowel marks may be precomposed (`ế`) or combining (`e` + U+0302 + U+0301).
fn parse(s: &str) -> Option<(String, usize)> {
    let mut letters: Vec<char> = Vec::new();
    let (mut tone, mut tones) = (0, 0);
    for c in s.chars() {
        let mark = match c {
            '\u{0300}' => 1,
            '\u{0301}' => 2,
            '\u{0309}' => 3,
            '\u{0303}' => 4,
            '\u{0323}' => 5,
            _ => 0,
        };
        if mark > 0 {
            (tone, tones) = (mark, tones + 1);
            continue;
        }
        if matches!(c, '\u{0302}' | '\u{0306}' | '\u{031B}') {
            let last = letters.last_mut()?;
            *last = match (*last, c) {
                ('a', '\u{0302}') => 'â',
                ('e', '\u{0302}') => 'ê',
                ('o', '\u{0302}') => 'ô',
                ('a', '\u{0306}') => 'ă',
                ('o', '\u{031B}') => 'ơ',
                ('u', '\u{031B}') => 'ư',
                _ => return None,
            };
            continue;
        }
        match VOWELS.iter().find_map(|v| Some((v.chars().next()?, v.chars().position(|x| x == c)?))) {
            Some((base, t)) => {
                letters.push(base);
                if t > 0 {
                    (tone, tones) = (t, tones + 1);
                }
            }
            None if matches!(c, 'b' | 'c' | 'd' | 'đ' | 'g' | 'h' | 'k' | 'l' | 'm' | 'n' | 'p' | 'q' | 'r' | 's' | 't' | 'v' | 'x') => {
                letters.push(c)
            }
            None => return None,
        }
    }
    (tones <= 1).then(|| (letters.into_iter().collect(), tone))
}

/// Do toneless letters `s` with `tone` follow the spelling rules: a known onset and rhyme, onsets
/// placed by the vowel after them, and a stop only with the acute or dot below?
fn valid(s: &str, tone: usize) -> bool {
    // Every onset that starts the syllable is tried: `gìn` is `g` + `in`, `gia` is `gi` + `a`.
    ONSETS.iter().filter_map(|o| Some((*o, s.strip_prefix(o)?))).chain([("", s)]).any(|(onset, rhyme)| {
        let front = rhyme.starts_with(['e', 'ê', 'i', 'y']);
        let placed = match onset {
            "k" | "gh" | "ngh" => front,
            "c" | "ng" => !front,
            "g" => !rhyme.starts_with(['e', 'ê', 'y']),
            _ => true,
        };
        // After `qu` the `u` is the rhyme's glide: `quân` is `qu` + `ân` of `uân`, `quen` is `qu` + `en`.
        let known = RHYMES.contains(&rhyme) || (onset == "qu" && RHYMES.contains(&format!("u{rhyme}").as_str()));
        let stop = rhyme.ends_with(['c', 'p', 't']) || rhyme.ends_with("ch");
        placed && known && (!stop || tone == 2 || tone == 5)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vietnamese_words() {
        for w in ["Việt", "ơn", "những", "Ước", "mũi", "Nghĩa", "tie\u{0302}\u{0301}ng", "nho\u{031B}"] {
            assert!(is_vietnamese_word(w), "{w}");
        }
        // Shared with French, Portuguese, Spanish: not enough on their own.
        // Romanian ă and Croatian đ are Vietnamese letters too, but not Vietnamese-only.
        for w in ["café", "être", "côte", "água", "señor", "anh", "the", "mănânc", "acasă", "Đurđa", "đak", "đi"] {
            assert!(!is_vietnamese_word(w), "{w}");
        }
    }

    #[test]
    fn paragraphs() {
        assert!(looks_vietnamese("Tôi yêu tiếng Việt và những bài thơ của nó."));
        assert!(looks_vietnamese("Hà Nội"));
        assert!(looks_vietnamese("Việt"));
        // English quoting one Vietnamese name, and French, stay as they are.
        assert!(!looks_vietnamese("The meeting with Mr Nguyễn is on Tuesday at the main office."));
        assert!(!looks_vietnamese("Le café de la côte est très bon."));
        assert!(!looks_vietnamese(""));
        assert!(!looks_vietnamese("anh em"));
        // Romanian and Croatian paragraphs, full of ă and đ, aren't Vietnamese.
        assert!(!looks_vietnamese("Mă duc acasă să mănânc cu fratele meu, că e târziu."));
        assert!(!looks_vietnamese("Đurđa i Đorđe su se vratili kući, a Đakovo je daleko."));
    }

    #[test]
    fn syllables() {
        for w in [
            "Việt",
            "Nam",
            "tiếng",
            "nghiêng",
            "người",
            "những",
            "quốc",
            "quân",
            "quen",
            "quyết",
            "Quỳnh",
            "khuỷu",
            "giường",
            "gì",
            "gìn",
            "giếng",
            "giữa",
            "Nguyễn",
            "học",
            "đẹp",
            "thuở",
            "khuya",
            "xoay",
            "huỳnh",
            "yêu",
            "ước",
            "ở",
            "à",
            "kỹ",
            "kĩ",
            "boong",
            "hoạch",
            "nguệch",
            "khuấy",
            "ngoằn",
            "ngoèo",
            "rượu",
            "tie\u{0302}\u{0301}ng",
            "nho\u{031B}",
            "ĐẸP",
            "Hà-Nội",
        ] {
            assert!(is_correct_with(w, &Syllables::new()), "{w}");
        }
        // Wrong onset before the vowel, a stop without acute or dot below, an impossible rhyme,
        // two tones, leftover Telex keys, letters Vietnamese doesn't use.
        for w in ["nghành", "ngỉ", "cinh", "ghà", "kà", "gé", "hoc", "hòc", "tiep", "uong", "tooi", "ă", "hóà", "tieengs", "ddi", "vieejt", "fhá"]
        {
            assert!(!is_correct_with(w, &Syllables::new()), "{w}");
        }
        // Everyday English words in Vietnamese text are let through.
        for w in ["email", "file", "online"] {
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn syllables_in_use() {
        // Comments, blank lines and rule-breaking entries are skipped; case and decomposition don't matter.
        let known = parse_list("# list\n\nhoà\nHòa\nQuý\nhoc\nquí\ntie\u{0302}\u{0301}ng\n");
        // Both tone placements share a key: `hoà` and `hòa` are `hoa` with the grave.
        assert_eq!(known.len(), 4);
        for w in ["hoà", "hòa", "quý", "Quí", "tiếng"] {
            assert!(is_correct_with(w, &known), "{w}");
        }
        // Rule-valid but not on the list; with no list the rules alone decide.
        assert!(!is_correct_with("quyêm", &known) && is_correct_with("quyêm", &Syllables::new()));
        // A list file is read; a missing one isn't.
        let path = std::env::temp_dir().join(format!("wordcraft-{}-{SYLLABLES_FILE}", std::process::id()));
        std::fs::write(&path, "nam\n").unwrap();
        assert_eq!(load(&path), Some(Syllables::from([("nam".to_string(), 0)])));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(load(&path), None);
    }
}

#[cfg(test)]
mod checker_tests {
    use crate::{Options, ProofLang, check_text_with};

    #[test]
    fn vietnamese_paragraphs_under_english_are_spelled_as_vietnamese() {
        let text = "Hôm nay tôi đi học, trời rất đẹp và mát mẻ.";
        let spans = [(0..text.len(), ProofLang::En)];
        assert!(check_text_with(text, &spans, &Options::default(), true).is_empty());
        // No language at all (the document default) behaves the same.
        assert!(check_text_with(text, &[], &Options::default(), true).is_empty());
        // A slip is caught, in Vietnamese.
        let text = "Hôm nay tôi đi hoc ở trường, ngành nghề rất đẹp.";
        let issues = check_text_with(text, &[], &Options::default(), true);
        let flagged: Vec<&str> = issues.iter().filter_map(|i| text.get(i.start..i.end)).collect();
        assert_eq!(flagged, ["hoc"]);
        assert!(issues.iter().all(|i| i.lang == ProofLang::Vi));
    }

    #[test]
    fn english_keeps_its_marks_and_skips_vietnamese_names() {
        let text = "We met Nguyễn yesterdy at the office.";
        let issues = check_text_with(text, &[(0..text.len(), ProofLang::En)], &Options::default(), false);
        let flagged: Vec<&str> = issues.iter().filter_map(|i| text.get(i.start..i.end)).collect();
        assert_eq!(flagged, ["yesterdy"]);
    }
}
