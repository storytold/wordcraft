//! The `vos` forms of verbs in Uruguayan Spanish ([`crate::ProofLang::EsUy`]), which dictionaries
//! built around `tú` may lack (#3360). Rioplatense `voseo` builds them regularly from the
//! infinitive, stress on the ending, without the stem changes `tú` has:
//!
//! - the imperative drops the infinitive's `r` and accents the vowel: `hablar` → `hablá`,
//!   `tener` → `tené`, `venir` → `vení`, `decir` → `decí` (`ir` uses `andá`, from `andar`);
//! - with one pronoun after it the accent goes (`decime`, `fijate`, `andate`, `poneme`), with two
//!   it comes back (`decímelo`, `mandámela`); `-ír` verbs keep theirs (`sonreíte`);
//! - the present indicative ends in `-ás`, `-és`, `-ís`: `hablás`, `tenés`, `venís`.
//!
//! A form is accepted only when the dictionary has its verb's infinitive, so nothing but these
//! endings is hard-coded. Other `vos` forms (`sos`) are ordinary dictionary words.

/// Pronouns that come first when two follow an imperative (`decímelo`, `dáselo`).
const FIRST_PRONOUNS: [&str; 4] = ["me", "te", "se", "nos"];
/// Pronouns that come second (or alone).
const SECOND_PRONOUNS: [&str; 6] = ["lo", "la", "le", "los", "las", "les"];

/// Shortest infinitive considered (`dar` and `ir` have no `vos` imperative of their own here, and
/// short stems would let nouns such as `bar` pass for verbs).
const MIN_INFINITIVE: usize = 4;

/// Is `word` (one word, any case) a `vos` form of a verb whose infinitive `is_word` accepts?
pub fn is_vos_form(word: &str, is_word: impl Fn(&str) -> bool) -> bool {
    let w = word.to_lowercase();
    if w.chars().count() > crate::affix::MAX_WORD_BYTES {
        return false;
    }
    infinitives(&w).iter().any(|inf| inf.chars().count() >= MIN_INFINITIVE && is_word(inf))
}

/// The infinitives `w` (lower case) could be a `vos` form of, by its ending.
fn infinitives(w: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Present indicative: `hablás`, `tenés`, `venís`.
    if let Some(stem) = w.strip_suffix('s') {
        imperative(stem, &mut out);
    }
    // Imperative: `hablá`, `tené`, `vení`.
    imperative(w, &mut out);
    // With one pronoun the accent goes: `decime`, `fijate`; `-ír` verbs keep it: `sonreíte`.
    for p in FIRST_PRONOUNS.iter().chain(SECOND_PRONOUNS.iter()) {
        if let Some(base) = w.strip_suffix(p)
            && base.ends_with(['a', 'e', 'i', 'í'])
        {
            out.push(format!("{base}r"));
        }
    }
    // With two the accent comes back: `decímelo`, `mandámela`.
    for second in SECOND_PRONOUNS {
        let Some(rest) = w.strip_suffix(second) else { continue };
        for first in FIRST_PRONOUNS {
            if let Some(base) = rest.strip_suffix(first) {
                imperative(base, &mut out);
            }
        }
    }
    out
}

/// The infinitives of an imperative ending in an accented vowel: `hablá` → `hablar`, and for `í`
/// both `vení` → `venir` and `sonreí` → `sonreír`.
fn imperative(form: &str, out: &mut Vec<String>) {
    let mut chars = form.chars();
    let Some(last) = chars.next_back() else { return };
    let stem = chars.as_str();
    match last {
        'á' => out.push(format!("{stem}ar")),
        'é' => out.push(format!("{stem}er")),
        'í' => {
            out.push(format!("{stem}ir"));
            out.push(format!("{stem}ír"));
        }
        _ => {}
    }
}

/// The `vos` form `word` (lower case, without its accent) was most likely meant as, when the
/// writer left the accent out: `veni` → `vení`, `tenes` → `tenés`. `None` when it isn't one.
pub fn accented(word: &str, is_word: impl Fn(&str) -> bool) -> Option<String> {
    let (body, s) = match word.strip_suffix('s') {
        Some(b) => (b, "s"),
        None => (word, ""),
    };
    let mut chars = body.chars();
    let accent = match chars.next_back()? {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        _ => return None,
    };
    let form = format!("{}{accent}{s}", chars.as_str());
    is_vos_form(&form, is_word).then_some(form)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verbs(w: &str) -> bool {
        ["hablar", "tener", "venir", "decir", "andar", "mirar", "sonreír", "fijar", "mandar", "poner", "bar", "querer"].contains(&w)
    }

    #[test]
    fn imperatives_present_and_pronouns_come_from_the_infinitive() {
        for w in [
            "hablá",
            "tené",
            "vení",
            "decí",
            "andá",
            "mirá",
            "poné",
            "hablás",
            "tenés",
            "venís",
            "querés",
            "decime",
            "fijate",
            "andate",
            "poneme",
            "sonreíte",
            "sonreí",
            "decímelo",
            "mandámela",
            "decinos",
            "Vení",
            "TENÉS",
        ] {
            assert!(is_vos_form(w, verbs), "{w}");
        }
        // Wrong accents, verbs the dictionary lacks, and short stems (`bame` isn't `bar`).
        for w in ["habla", "decíme", "decínos", "decimelo", "tenéz", "cantá", "bá", "bame", "", "s", "á", "més", "lo", "melo"] {
            assert!(!is_vos_form(w, verbs), "{w}");
        }
    }

    #[test]
    fn missing_accents_are_restored() {
        assert_eq!(accented("veni", verbs).as_deref(), Some("vení"));
        assert_eq!(accented("tenes", verbs).as_deref(), Some("tenés"));
        assert_eq!(accented("habla", verbs).as_deref(), Some("hablá"));
        for w in ["canta", "casa", "", "s", "pan", "tenés"] {
            assert_eq!(accented(w, verbs), None, "{w}");
        }
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "\\PC{0,40}") {
            let _ = is_vos_form(&s, |_| true);
            let _ = accented(&s, |_| true);
        }
    }
}
