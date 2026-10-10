//! Proofing languages: which dictionary, grammar rules and hyphenation patterns apply to a run of
//! text. A run's language is its BCP 47 tag (`w:lang` in .docx, `CharProps::lang` in the model),
//! resolved with [`ProofLang::from_tag`].

/// A language WordCraft can proof, or [`ProofLang::Other`] for one it can't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProofLang {
    /// English, any region (`en`, `en-US`, `en-GB`…): the public-domain Moby word list, the
    /// English grammar rules and the English hyphenation dictionary and patterns. Also text with
    /// no language at all (WordCraft's default editing language is US English, like Word's).
    #[default]
    En,
    /// Polish (`pl`, `pl-PL`): the SJP.PL dictionary, the Polish grammar rules and the Polish
    /// hyphenation patterns.
    Pl,
    /// A language without proofing tools here (`fr-FR`, `ja-JP`, `x-none`…). Like Word without
    /// the language's proofing tools: no spelling or grammar marks, so its words are never
    /// flagged as English misspellings. Hyphenation keeps using the English patterns, as before
    /// languages were told apart.
    Other,
}

impl ProofLang {
    /// Every language with a spelling dictionary, in display order.
    pub const PROOFED: [ProofLang; 2] = [ProofLang::En, ProofLang::Pl];

    /// The proofing language of a BCP 47 tag (`pl-PL`, `en_GB`, `PL`…). Only the primary subtag
    /// counts. No tag (or an empty one) is English, WordCraft's default editing language.
    pub fn from_tag(tag: Option<&str>) -> ProofLang {
        let Some(tag) = tag.map(str::trim).filter(|t| !t.is_empty()) else { return ProofLang::En };
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("en") || primary.eq_ignore_ascii_case("eng") {
            ProofLang::En
        } else if primary.eq_ignore_ascii_case("pl") || primary.eq_ignore_ascii_case("pol") {
            ProofLang::Pl
        } else {
            ProofLang::Other
        }
    }

    /// The primary language subtag (`en`, `pl`); empty for [`ProofLang::Other`].
    pub fn code(self) -> &'static str {
        match self {
            ProofLang::En => "en",
            ProofLang::Pl => "pl",
            ProofLang::Other => "",
        }
    }

    /// Does WordCraft check spelling and grammar in this language?
    pub fn is_proofed(self) -> bool {
        self != ProofLang::Other
    }
}

/// A BCP 47 tag in canonical case (`pl-PL`, `en-GB`, `zh-Hant-TW`, `x-none`), or `None` when
/// `tag` isn't shaped like one: 1–35 ASCII letters, digits and `-`/`_` separators, starting with
/// a 2–8 letter language (or `x`/`i` for private and grandfathered tags). `_` becomes `-`.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if tag.is_empty() || tag.len() > 35 || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let parts: Vec<&str> = tag.split(['-', '_']).collect();
    if parts.iter().any(|p| p.is_empty() || p.len() > 8) {
        return None;
    }
    let first = parts.first()?;
    let language = (2..=8).contains(&first.len()) && first.chars().all(|c| c.is_ascii_alphabetic());
    if !language && !first.eq_ignore_ascii_case("x") && !first.eq_ignore_ascii_case("i") {
        return None;
    }
    let letters = |p: &str| p.chars().all(|c| c.is_ascii_alphabetic());
    // After a singleton (`x-…` private use, `u-…` extensions) subtags are lower case.
    let singleton = parts.iter().position(|p| p.len() == 1).unwrap_or(usize::MAX);
    let out: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i >= singleton {
                p.to_ascii_lowercase()
            } else if i > 0 && p.len() == 2 && letters(p) {
                p.to_ascii_uppercase() // region
            } else if i > 0 && p.len() == 4 && letters(p) {
                let lower = p.to_ascii_lowercase();
                let mut c = lower.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default() // script
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect();
    Some(out.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_normalize() {
        assert_eq!(normalize_tag("PL-pl").as_deref(), Some("pl-PL"));
        assert_eq!(normalize_tag(" pl_pl ").as_deref(), Some("pl-PL"));
        assert_eq!(normalize_tag("pl").as_deref(), Some("pl"));
        assert_eq!(normalize_tag("zh-hant-tw").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(normalize_tag("es-419").as_deref(), Some("es-419"));
        assert_eq!(normalize_tag("x-none").as_deref(), Some("x-none"));
        assert_eq!(normalize_tag("de-DE-u-co-PHONEBK").as_deref(), Some("de-DE-u-co-phonebk"));
        for bad in ["", " ", "p", "1pl", "pl--PL", "pl PL", "pl-PL;", "polski język", "-pl", "pl-", "abcdefghi", &"a".repeat(40)] {
            assert_eq!(normalize_tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn tags_resolve() {
        for t in ["pl", "pl-PL", "PL-pl", "pl_PL", "pl_PL.UTF-8", " pl-PL ", "pol"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Pl, "{t}");
        }
        for t in ["en", "en-US", "en-GB", "EN_au", "en-Latn-US"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::En, "{t}");
        }
        for t in ["fr-FR", "de", "x-none", "ja-JP", "plx", "e", "-"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Other, "{t}");
        }
        assert_eq!(ProofLang::from_tag(None), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("")), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("  ")), ProofLang::En);
        assert!(ProofLang::Pl.is_proofed() && !ProofLang::Other.is_proofed());
        assert_eq!(ProofLang::Pl.code(), "pl");
    }
}
