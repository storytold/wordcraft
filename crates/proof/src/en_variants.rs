//! American and British spellings of the same English word (#1281, #1045): which forms each
//! English ([`ProofLang::En`], [`ProofLang::EnCa`], [`ProofLang::EnGb`]) accepts, which it flags,
//! and what it suggests instead (`colour` → `color` in American English).
//!
//! The pairs come from `assets/spelling/en-variants.txt`, our own list of base forms with a
//! policy per English. Inflections are generated here by rule for the regular patterns (-or/-our,
//! -er/-re, -ize/-ise, -ense/-ence…), so `colours`, `centred` and `organisation` follow their
//! base forms. A form is right when the shared English word list knows it or knows its other
//! spelling (`realisation` is right in British English because `realization` is a word), so the
//! rules never make a misspelling right.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::ProofLang;

/// Our list of American/British pairs: `POLICY AMERICAN BRITISH` per line, `#` comments.
const VARIANTS: &str = include_str!("../../../assets/spelling/en-variants.txt");

/// What one English does with a pair's two spellings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    /// Only the American spelling is right.
    American,
    /// Only the British spelling is right.
    British,
    /// Both are right.
    Both,
}

impl Policy {
    fn from_char(c: char) -> Option<Policy> {
        match c {
            'u' => Some(Policy::American),
            'b' => Some(Policy::British),
            '*' => Some(Policy::Both),
            _ => None,
        }
    }
}

/// One line of the list: the policies for American, Canadian and British-spelling English, and
/// the two base forms.
struct Pair {
    policy: [Policy; 3],
    american: &'static str,
    british: &'static str,
}

/// The lines of [`VARIANTS`]; malformed lines are skipped (a test keeps the file clean).
fn pairs() -> &'static [Pair] {
    static P: OnceLock<Vec<Pair>> = OnceLock::new();
    P.get_or_init(|| VARIANTS.lines().filter_map(parse_line).collect())
}

fn parse_line(line: &'static str) -> Option<Pair> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split_whitespace();
    let (policy, american, british) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || american == british {
        return None;
    }
    let mut p = policy.chars().map(Policy::from_char);
    let policy = [p.next()??, p.next()??, p.next()??];
    if p.next().is_some() {
        return None;
    }
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase());
    (word(american) && word(british)).then_some(Pair { policy, american, british })
}

/// The column of a pair's policy for `lang`; `None` for a language that isn't English.
fn column(lang: ProofLang) -> Option<usize> {
    match lang {
        ProofLang::En => Some(0),
        ProofLang::EnCa => Some(1),
        ProofLang::EnGb => Some(2),
        _ => None,
    }
}

/// `word` with `suffix`: a final `e` drops before a vowel (`centre` + `ing` → `centring`), a
/// final consonant + `y` becomes `i` (`cosy` + `er` → `cosier`), `s` is the plural (`es` after a
/// sibilant, `ies` after a consonant + `y`).
fn add(word: &str, suffix: &str) -> String {
    let b = word.as_bytes();
    let last = b.last().copied().unwrap_or(b'_');
    let before = b.len().checked_sub(2).and_then(|i| b.get(i)).copied().unwrap_or(b'_');
    let vowel = |c: u8| b"aeiou".contains(&c);
    let head = word.get(..word.len().saturating_sub(1)).unwrap_or(word);
    if suffix == "s" {
        if last == b'y' && !vowel(before) {
            return format!("{head}ies");
        }
        if matches!(last, b's' | b'x' | b'z') || word.ends_with("ch") || word.ends_with("sh") {
            return format!("{word}es");
        }
        return format!("{word}s");
    }
    let suffix_vowel = suffix.as_bytes().first().is_some_and(|&c| vowel(c));
    if last == b'e' && suffix_vowel {
        return format!("{head}{suffix}");
    }
    if last == b'y' && !vowel(before) && !suffix.starts_with('i') {
        return format!("{head}i{suffix}");
    }
    format!("{word}{suffix}")
}

/// The suffixes both spellings of a pair take, by the pattern that tells them apart.
fn suffixes(american: &str, british: &str) -> &'static [&'static str] {
    // Not -al, -ist or -ite: `humoral` and `humorist` are spelled so in British English too, and
    // `behavioural` and `favourite` have lines of their own.
    const OUR: &[&str] = &["s", "ed", "ing", "er", "ers", "ful", "fully", "less", "able", "ably"];
    const RE: &[&str] = &["s", "ed", "ing"];
    const IZE: &[&str] = &["s", "ed", "ing", "er", "ers", "ation", "ations", "ational", "able"];
    const E: &[&str] = &["s", "ed", "ing", "er", "ers"];
    const Y: &[&str] = &["s", "er", "est", "ly", "ness"];
    const PLURAL: &[&str] = &["s"];
    let stem = |w: &str, n: usize| w.get(..w.len().saturating_sub(n)).map(str::to_string);
    if american.ends_with("or") && british.ends_with("our") && stem(american, 2) == stem(british, 3) {
        return OUR;
    }
    if american.ends_with("er") && british.ends_with("re") && stem(american, 2) == stem(british, 2) {
        return RE;
    }
    // One letter apart (`organize`/`organise`, `defense`/`defence`, `cozy`/`cosy`).
    let differ = american.len() == british.len() && american.bytes().zip(british.bytes()).filter(|(a, b)| a != b).count() == 1;
    if differ && american.ends_with('e') && british.ends_with('e') {
        let ize = american.ends_with("ize") || american.ends_with("yze");
        return if ize { IZE } else { E };
    }
    if differ && american.ends_with('y') && british.ends_with('y') {
        return Y;
    }
    PLURAL
}

/// One English's forms: those it flags (each with the form to suggest), and those it accepts
/// when the shared list knows them or their other spelling (each with that other spelling).
#[derive(Default)]
struct Forms {
    foreign: HashMap<String, String>,
    accepted: HashMap<String, String>,
}

fn forms(lang: ProofLang) -> Option<&'static Forms> {
    static F: [OnceLock<Forms>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let col = column(lang)?;
    Some(F.get(col)?.get_or_init(|| build(col)))
}

fn build(col: usize) -> Forms {
    let mut f = Forms::default();
    for pair in pairs() {
        let Some(&policy) = pair.policy.get(col) else { continue };
        let mut both = vec![(pair.american.to_string(), pair.british.to_string())];
        both.extend(suffixes(pair.american, pair.british).iter().map(|s| (add(pair.american, s), add(pair.british, s))));
        for (am, br) in both {
            if am == br {
                continue;
            }
            match policy {
                Policy::American => {
                    f.foreign.insert(br.clone(), am.clone());
                    f.accepted.insert(am, br);
                }
                Policy::British => {
                    f.foreign.insert(am.clone(), br.clone());
                    f.accepted.insert(br, am);
                }
                Policy::Both => {
                    f.accepted.insert(am.clone(), br.clone());
                    f.accepted.insert(br, am);
                }
            }
        }
    }
    // A form one pair accepts is never flagged by another, and `analyses`, `paralyses`… are also
    // the plurals of `analysis`, `paralysis`… everywhere.
    f.foreign.retain(|w, _| !f.accepted.contains_key(w) && !w.ends_with("yses"));
    f
}

/// `lower` without a possessive `'s`.
fn without_possessive(lower: &str) -> &str {
    lower.strip_suffix("'s").filter(|s| !s.is_empty()).unwrap_or(lower)
}

/// The spelling `lang` wants instead of `lower` (a lower-case word, `'` apostrophes), when
/// `lower` is the other dialect's spelling of a word (`colour` in American English: `color`).
/// A possessive keeps its `'s` (`colour's` → `color's`).
pub fn preferred(lower: &str, lang: ProofLang) -> Option<String> {
    let f = forms(lang)?;
    let base = without_possessive(lower);
    let native = f.foreign.get(base)?;
    Some(if base.len() < lower.len() { format!("{native}'s") } else { native.clone() })
}

/// Is `lower` right in `lang` because the shared word list knows its other spelling (`known`
/// says what the list knows)? `realisation` in British English, `organization` in all three.
pub fn known_variant(lower: &str, lang: ProofLang, known: impl Fn(&str) -> bool) -> bool {
    let Some(f) = forms(lang) else { return false };
    let base = without_possessive(lower);
    f.accepted.get(base).is_some_and(|other| known(other))
}

/// Is `lower` the other dialect's spelling in `lang` (flagged even though it is a word)?
pub fn is_foreign(lower: &str, lang: ProofLang) -> bool {
    forms(lang).is_some_and(|f| f.foreign.contains_key(without_possessive(lower)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_parses() {
        let lines = VARIANTS.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).count();
        assert_eq!(pairs().len(), lines, "a malformed line in en-variants.txt");
        assert!(pairs().len() > 250);
    }

    #[test]
    fn suffixes_follow_the_pattern() {
        assert_eq!(add("centre", "ing"), "centring");
        assert_eq!(add("centre", "ed"), "centred");
        assert_eq!(add("center", "ed"), "centered");
        assert_eq!(add("organise", "ation"), "organisation");
        assert_eq!(add("cosy", "er"), "cosier");
        assert_eq!(add("cosy", "s"), "cosies");
        assert_eq!(add("armoury", "s"), "armouries");
        assert_eq!(add("analyse", "s"), "analyses");
        assert_eq!(add("colour", "ful"), "colourful");
        assert_eq!(add("", "s"), "s");
        assert_eq!(suffixes("color", "colour").len(), 10);
        assert_eq!(suffixes("center", "centre"), ["s", "ed", "ing"]);
        assert!(suffixes("organize", "organise").contains(&"ation"));
        assert_eq!(suffixes("program", "programme"), ["s"], "no -ed: programmed is American too");
    }

    #[test]
    fn no_form_is_both_flagged_and_accepted() {
        for lang in [ProofLang::En, ProofLang::EnCa, ProofLang::EnGb] {
            let Some(f) = forms(lang) else { panic!("{lang:?}") };
            for (w, native) in &f.foreign {
                assert!(!f.accepted.contains_key(w), "{lang:?}: {w}");
                assert!(!f.foreign.contains_key(native), "{lang:?}: suggests {native}, itself flagged");
            }
        }
        // Words both dialects use alike are never flagged.
        for w in [
            "programmed",
            "tired",
            "tires",
            "curbs",
            "humorous",
            "honorary",
            "humorist",
            "humoral",
            "meter",
            "license",
            "for",
            "tour",
            "analyses",
            "paralyses",
        ] {
            for lang in [ProofLang::En, ProofLang::EnCa, ProofLang::EnGb] {
                assert!(!is_foreign(w, lang), "{lang:?}: {w}");
            }
        }
        assert!(forms(ProofLang::Pl).is_none() && !is_foreign("color", ProofLang::Other));
    }
}
