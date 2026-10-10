//! German spelling: the word forms of the German word list ([`WordList::german`]), German
//! capitalisation (nouns are listed capitalised, so `haus` is flagged as Word flags it; any word
//! may be capitalised at the start of a sentence), words in capitals (`STRASSE`), Swiss `ss` for
//! `ß`, and compounds, which German writes as one word: `Haushalts|budget`, `Sonne|n|schein`,
//! `Schul(e)|buch`, `dunkel|blau`.

use crate::wordlist::WordList;

/// Shortest part of a compound, in characters (keeps `Ab|ende`-style splits out).
const MIN_PART: usize = 3;
/// Longest word looked at as a compound, in characters.
const MAX_COMPOUND: usize = 64;
/// How many parts deep compounds are split.
const MAX_DEPTH: u8 = 4;
/// Linking letters between the parts of a compound (`Arbeit|s|zimmer`, `Kind|er|garten`).
const LINKS: [&str; 7] = ["s", "es", "n", "en", "er", "e", "ns"];

fn listed(w: &str) -> bool {
    WordList::german().contains(w) || crate::user_knows(w)
}

/// `w` with its first letter in upper case (`budget` → `Budget`).
fn capitalized(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// `w` with its first letter in lower case (`Aber` → `aber`).
fn decapitalized(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

fn all_caps(w: &str) -> bool {
    w.chars().filter(|c| c.is_alphabetic()).count() > 1 && !w.chars().any(char::is_lowercase)
}

/// `w` with one or all `ss` written `ß` (`Strasse` → `Straße`), at most a few variants.
fn eszett_variants(w: &str) -> Vec<String> {
    let mut v = Vec::new();
    for (i, _) in w.match_indices("ss").take(4) {
        if let (Some(a), Some(b)) = (w.get(..i), w.get(i + 2..)) {
            v.push(format!("{a}ß{b}"));
        }
    }
    if v.len() > 1 {
        v.push(w.replace("ss", "ß"));
    }
    v
}

/// A listed form, or a listed lower-case word capitalised (`Aber` at the start of a sentence).
fn known_form(w: &str) -> bool {
    listed(w) || (w.chars().next().is_some_and(char::is_uppercase) && listed(&decapitalized(w)))
}

/// A word as written: a form, a word in capitals, Swiss `ss`, or a compound.
fn known(w: &str, swiss: bool) -> bool {
    if known_form(w) {
        return true;
    }
    // Capitals: `HAUS` is `Haus` or `haus`; `STRASSE` is `Straße` (capital ß is rare).
    let (w, eszett) = if all_caps(w) {
        let lower = w.to_lowercase();
        (capitalized(&lower), true)
    } else {
        (w.to_string(), swiss)
    };
    if known_form(&w) {
        return true;
    }
    if eszett && eszett_variants(&w).iter().any(|v| known_form(v)) {
        return true;
    }
    // Negated adjectives and participles: `un|beschriftet`, `Un|gelesene`.
    if let Some(rest) = w.strip_prefix("un").or_else(|| w.strip_prefix("Un"))
        && rest.chars().count() >= 4
        && rest.chars().next().is_some_and(char::is_lowercase)
        && known_form(rest)
    {
        return true;
    }
    let mut split = Split { swiss: eszett, budget: LOOKUP_BUDGET };
    split.compound(&w, 0)
}

/// Word-list lookups one compound may cost: long nonsense words give up instead of trying
/// every split.
const LOOKUP_BUDGET: u32 = 4_000;

struct Split {
    swiss: bool,
    budget: u32,
}

impl Split {
    fn spend(&mut self) -> bool {
        self.budget = self.budget.saturating_sub(1);
        self.budget > 0
    }

    fn listed(&mut self, w: &str) -> bool {
        self.spend() && (listed(w) || (self.swiss && eszett_variants(w).iter().any(|v| listed(v))))
    }

    fn form(&mut self, w: &str) -> bool {
        self.spend() && (known_form(w) || (self.swiss && eszett_variants(w).iter().any(|v| known_form(v))))
    }

    /// A compound of known words: the last part is a word (a noun, capitalised, in a
    /// capitalised compound), the parts before it are words or word stems with a linking letter.
    fn compound(&mut self, w: &str, depth: u8) -> bool {
        if depth >= MAX_DEPTH || self.budget == 0 {
            return false;
        }
        let starts: Vec<usize> = w.char_indices().map(|(i, _)| i).collect();
        let n = starts.len();
        if !(2 * MIN_PART..=MAX_COMPOUND).contains(&n) {
            return false;
        }
        let noun = w.chars().next().is_some_and(char::is_uppercase);
        // Longest last part first: `Haus|haltsbudget` is tried before `Haushalts|budget`.
        for k in MIN_PART..=n - MIN_PART {
            let Some(&b) = starts.get(k) else { continue };
            let (Some(head), Some(tail)) = (w.get(..b), w.get(b..)) else { continue };
            if tail.starts_with('-') || head.ends_with('-') {
                continue;
            }
            let tail = if noun { capitalized(tail) } else { tail.to_string() };
            if (self.listed(&tail) || self.compound(&tail, depth + 1)) && self.head(head, depth) {
                return true;
            }
            if self.budget == 0 {
                return false;
            }
        }
        false
    }

    /// The front part of a compound: a word (`Haus`, `schnell` in `Schnellzug`), a word with a
    /// linking letter (`Arbeit|s`), a word that drops its final `e` (`Schul` for `Schule`), or
    /// a compound itself.
    fn head(&mut self, head: &str, depth: u8) -> bool {
        if self.form(head) || self.form(&format!("{head}e")) {
            return true;
        }
        for l in LINKS {
            if let Some(h) = head.strip_suffix(l)
                && h.chars().count() >= MIN_PART
                && self.form(h)
            {
                return true;
            }
        }
        self.compound(head, depth + 1)
    }
}

/// Is a German word (already trimmed; not a number, an acronym or a single letter) correct?
pub(crate) fn is_correct(w: &str, swiss: bool) -> bool {
    let w = w.replace('’', "'");
    if known(&w, swiss) {
        return true;
    }
    // `geht's`, `gibt's`.
    if let Some(stem) = w.strip_suffix("'s")
        && known(stem, swiss)
    {
        return true;
    }
    // Hyphenated compounds: each part (`E-Mail-Adresse`, `Software-Entwicklung`).
    if w.contains('-') {
        return w.split('-').all(|p| p.chars().count() <= 1 || p.chars().any(|c| c.is_ascii_digit()) || known(p, swiss));
    }
    false
}

/// Suggestions for a misspelled German word, best first.
pub(crate) fn suggest(word: &str, max: usize) -> Vec<String> {
    let target: Vec<char> = word.to_lowercase().chars().collect();
    let n = target.len();
    if n == 0 || n > 40 {
        return Vec::new();
    }
    let cap = word.chars().next().is_some_and(char::is_uppercase);
    let upper = all_caps(word);
    let mut scored: Vec<(usize, u8, usize, String)> = Vec::new();
    for w in WordList::german().iter() {
        let wn = w.chars().count();
        if wn + 2 < n || wn > n + 2 {
            continue;
        }
        let lower = w.to_lowercase();
        let dist = crate::damerau(&target, &lower);
        if dist <= 2 {
            let same_first = lower.chars().next() == target.first().copied();
            // Typos rarely change a word's last letter (`Fehlr` → `Fehler`, not `Fehle`).
            let same_last = lower.chars().last() == target.last().copied();
            // Prefer the form whose capitalisation matches what was typed.
            let case_match = w.chars().next().is_some_and(char::is_uppercase) == cap;
            let rank = u8::from(!same_first) * 4 + u8::from(!same_last) * 2 + u8::from(!case_match);
            scored.push((dist, rank, wn.abs_diff(n), w.to_string()));
        }
    }
    scored.sort();
    let mut out: Vec<String> = Vec::new();
    for (_, _, _, w) in scored {
        let w = if upper && n > 1 {
            w.to_uppercase()
        } else if cap {
            capitalized(&w)
        } else {
            w
        };
        if !out.contains(&w) && w != word {
            out.push(w);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_capitals_and_compounds() {
        for w in [
            "Haus",
            "Häuser",
            "gehen",
            "ging",
            "Aber",
            "HAUS",
            "STRASSE",
            "Haushaltsbudget",
            "Haushaltsbudgets",
            "Sonnenschein",
            "Schulbuch",
            "Arbeitszimmer",
            "dunkelblau",
            "Schnellzug",
            "Kinderzimmerlampe",
            "E-Mail-Adresse",
            "Software-Entwicklung",
            "geht's",
            "gibt’s",
            "Unbeschriftetes",
            "ungelesen",
        ] {
            assert!(is_correct(w, false), "{w}");
        }
        // `katze` is a noun written in lower case (`haus` would pass: the imperative of `hausen`).
        // (`Strasse` would pass too: the old dative of `Strass`.)
        for w in ["katze", "Hasu", "Rechnug", "gegangt", "Fussball", "Xyzqwrt", "Hausxqz"] {
            assert!(!is_correct(w, false), "{w}");
        }
        // Swiss German writes ss for ß.
        assert!(is_correct("Fussball", true));
        assert!(is_correct("grossartig", true));
        assert!(is_correct("Fußball", false));
    }

    #[test]
    fn suggestions_keep_german_capitals() {
        let s = suggest("Rechnug", 5);
        assert_eq!(s.first().map(String::as_str), Some("Rechnung"), "{s:?}");
        let s = suggest("schoen", 5);
        assert!(s.iter().any(|w| w == "schön" || w == "schon"), "{s:?}");
        assert!(suggest("", 5).is_empty());
    }

    #[test]
    fn hostile_words_never_panic() {
        let long = "a".repeat(500);
        let nonsense = "Qwertzuiopasdfghjklyxcvbnmqwertzuiopasdfghjklyxcvbnmqwer";
        let t = std::time::Instant::now();
        assert!(!is_correct(nonsense, false));
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
        for w in ["", "-", "--", "'s", "ß", "SS", "ẞ", "İstanbul", long.as_str(), "Ab-", "-ab", "x'y's"] {
            let _ = is_correct(w, false);
            let _ = is_correct(w, true);
            let _ = suggest(w, 3);
        }
    }
}
