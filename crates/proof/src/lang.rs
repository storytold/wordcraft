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
    /// Polish (`pl`, `pl-PL`): the installed Hunspell dictionary (when there is one), the Polish grammar rules and the Polish
    /// hyphenation patterns.
    Pl,
    /// Bulgarian (`bg`, `bg-BG`): hyphenation by the Bulgarian syllable rules ([`crate::bg`]).
    /// There is no Bulgarian dictionary or grammar yet, so, as for [`ProofLang::Other`], no
    /// spelling or grammar marks.
    Bg,
    /// Portuguese (`pt`, `pt-BR`, `pt-PT`): the installed Hunspell dictionary (VERO and
    /// friends, when there is one), the Portuguese grammar rules and the Portuguese hyphenation
    /// patterns.
    Pt,
    /// Italian (`it`, `it-IT`, `it-CH`): the installed Hunspell dictionary (`it_IT`, when there
    /// is one), the Italian grammar rules and the Italian hyphenation patterns.
    It,
    /// Croatian (`hr`, `hr-HR`)
    Hr,
    /// Turkish (`tr`, `tr-TR`, `tr-CY`): the installed Hunspell dictionary (when there is one),
    /// Turkish grammar rules, spelling suggestions and Turkish syllable hyphenation rules.
    Tr,
    /// Russian (`ru`, `ru-RU`): the installed Hunspell dictionary (when there is one), the Russian
    /// grammar rules and the Russian word-division rules.
    Ru,
    /// Czech (`cs`, `cs-CZ`): the installed Hunspell dictionary, dictionary-driven suggestions
    /// and generic punctuation checks. Hyphenation keeps the fallback English patterns.
    Cs,
    /// Indonesian (`id`, `id-ID`; also the old `in` and ISO 639-2 `ind`): the installed Hunspell
    /// `id_ID` dictionary, dictionary-driven suggestions and generic checks with Indonesian
    /// messages. Hyphenation keeps the fallback English patterns.
    Id,
    /// Dutch (`nl`, `nl-NL`, `nl-BE`; also ISO 639-2 `nld` and `dut`): the installed Hunspell
    /// `nl_NL` dictionary (OpenTaal's, which covers Belgium too) with its compounding,
    /// dictionary-driven suggestions and generic checks with Dutch messages. Hyphenation keeps
    /// the fallback English patterns.
    Nl,
    /// Catalan (`ca`, `ca-ES`, `cat`): installed Hunspell spelling and suggestions only.
    /// Grammar is unavailable; hyphenation keeps the existing English fallback.
    Ca,
    /// A language without proofing tools here (`fr-FR`, `ja-JP`, `x-none`…). Like Word without
    /// the language's proofing tools: no spelling or grammar marks, so its words are never
    /// flagged as English misspellings. Hyphenation keeps using the English patterns, as before
    /// languages were told apart.
    Other,
}

impl ProofLang {
    /// Every language with a spelling dictionary, in display order.
    pub const PROOFED: [ProofLang; 11] = [
        ProofLang::En,
        ProofLang::Pl,
        ProofLang::Pt,
        ProofLang::It,
        ProofLang::Hr,
        ProofLang::Tr,
        ProofLang::Ru,
        ProofLang::Cs,
        ProofLang::Id,
        ProofLang::Nl,
        ProofLang::Ca,
    ];

    /// The proofing language of a BCP 47 tag (`pl-PL`, `en_GB`, `PL`…). Only the primary subtag
    /// counts. No tag (or an empty one) is English, WordCraft's default editing language.
    pub fn from_tag(tag: Option<&str>) -> ProofLang {
        let Some(tag) = tag.map(str::trim).filter(|t| !t.is_empty()) else { return ProofLang::En };
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("en") || primary.eq_ignore_ascii_case("eng") {
            ProofLang::En
        } else if primary.eq_ignore_ascii_case("pl") || primary.eq_ignore_ascii_case("pol") {
            ProofLang::Pl
        } else if primary.eq_ignore_ascii_case("bg") || primary.eq_ignore_ascii_case("bul") {
            ProofLang::Bg
        } else if primary.eq_ignore_ascii_case("pt") || primary.eq_ignore_ascii_case("por") {
            ProofLang::Pt
        } else if primary.eq_ignore_ascii_case("it") || primary.eq_ignore_ascii_case("ita") {
            ProofLang::It
        } else if primary.eq_ignore_ascii_case("hr") || primary.eq_ignore_ascii_case("hrv") {
            ProofLang::Hr
        } else if primary.eq_ignore_ascii_case("tr") || primary.eq_ignore_ascii_case("tur") {
            ProofLang::Tr
        } else if primary.eq_ignore_ascii_case("ru") || primary.eq_ignore_ascii_case("rus") {
            ProofLang::Ru
        } else if primary.eq_ignore_ascii_case("cs") || primary.eq_ignore_ascii_case("ces") || primary.eq_ignore_ascii_case("cze") {
            ProofLang::Cs
        } else if primary.eq_ignore_ascii_case("id") || primary.eq_ignore_ascii_case("ind") || primary.eq_ignore_ascii_case("in") {
            ProofLang::Id
        } else if primary.eq_ignore_ascii_case("nl") || primary.eq_ignore_ascii_case("nld") || primary.eq_ignore_ascii_case("dut") {
            ProofLang::Nl
        } else if primary.eq_ignore_ascii_case("ca") || primary.eq_ignore_ascii_case("cat") {
            ProofLang::Ca
        } else {
            ProofLang::Other
        }
    }

    /// The primary language subtag (`en`, `pl`, `bg`, `tr`, `ru`, `cs`, `id`, `nl`); empty for [`ProofLang::Other`].
    pub fn code(self) -> &'static str {
        match self {
            ProofLang::En => "en",
            ProofLang::Pl => "pl",
            ProofLang::Bg => "bg",
            ProofLang::Pt => "pt",
            ProofLang::It => "it",
            ProofLang::Hr => "hr",
            ProofLang::Tr => "tr",
            ProofLang::Ru => "ru",
            ProofLang::Cs => "cs",
            ProofLang::Id => "id",
            ProofLang::Nl => "nl",
            ProofLang::Ca => "ca",
            ProofLang::Other => "",
        }
    }

    /// Does WordCraft check spelling and grammar in this language?
    pub fn is_proofed(self) -> bool {
        Self::PROOFED.contains(&self)
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
        for t in ["pt", "pt-BR", "PT_br", "pt-BR.UTF-8", " pt-PT ", "por"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Pt, "{t}");
        }
        for t in ["it", "it-IT", "IT_it", "it-CH", "it-CH.UTF-8", " it-IT ", "ita"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::It, "{t}");
        }
        for t in ["cs", "cs-CZ", "CS-cz", "cs_CZ", " ces ", "CES", "cze", "CZE"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Cs, "{t}");
        }
        assert_eq!(ProofLang::Cs.code(), "cs");
        assert!(ProofLang::Cs.is_proofed() && ProofLang::PROOFED.contains(&ProofLang::Cs));
        for t in ["id", "id-ID", "ID-id", "id_ID", "id_ID.UTF-8", " id-ID ", "ind", "in", "in_ID"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Id, "{t}");
        }
        assert_eq!(ProofLang::Id.code(), "id");
        assert!(ProofLang::Id.is_proofed() && ProofLang::PROOFED.contains(&ProofLang::Id));
        for t in ["nl", "nl-NL", "nl-BE", "NL_nl", "nl_BE.UTF-8", " nl-NL ", "nld", "dut", "nl-SR"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Nl, "{t}");
        }
        assert_eq!(ProofLang::Nl.code(), "nl");
        assert!(ProofLang::Nl.is_proofed() && ProofLang::PROOFED.contains(&ProofLang::Nl));
        for t in ["ca", "ca-ES", "CA_es", "cat"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Ca, "{t}");
        }
        assert_eq!(ProofLang::Ca.code(), "ca");
        assert!(ProofLang::Ca.is_proofed());
        for t in ["en", "en-US", "en-GB", "EN_au", "en-Latn-US"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::En, "{t}");
        }
        for t in ["bg", "bg-BG", "BG_bg", "bg_BG.UTF-8", "bul"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Bg, "{t}");
        }
        for t in ["tr", "tr-TR", "tr-CY", "TR_tr", "tur"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Tr, "{t}");
        }
        for t in ["fr-FR", "de", "x-none", "ja-JP", "plx", "bgx", "trx", "csx", "idx", "inx", "nlx", "dutch", "e", "-"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Other, "{t}");
        }
        assert_eq!(ProofLang::from_tag(None), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("")), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("  ")), ProofLang::En);
        assert!(ProofLang::Pl.is_proofed() && !ProofLang::Other.is_proofed());
        // Bulgarian has hyphenation only: no spelling or grammar marks.
        assert!(!ProofLang::Bg.is_proofed());
        assert_eq!(ProofLang::Bg.code(), "bg");
        assert!(ProofLang::Pt.is_proofed() && !ProofLang::Other.is_proofed());
        assert!(ProofLang::Tr.is_proofed());
        for t in ["ru", "ru-RU", "RU-ru", "ru_RU", "ru_RU.UTF-8", " ru-RU ", "rus"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Ru, "{t}");
        }
        assert!(ProofLang::Ru.is_proofed());
        assert_eq!(ProofLang::Ru.code(), "ru");
        assert!(ProofLang::PROOFED.contains(&ProofLang::Ru));
        assert_eq!(ProofLang::Pl.code(), "pl");
        assert_eq!(ProofLang::Pt.code(), "pt");
        assert!(ProofLang::It.is_proofed());
        assert_eq!(ProofLang::It.code(), "it");
        assert!(ProofLang::Hr.is_proofed());
        assert_eq!(ProofLang::Hr.code(), "hr");
        assert_eq!(ProofLang::Tr.code(), "tr");
        assert!(ProofLang::PROOFED.contains(&ProofLang::Pt), "every proofed language is listed");
    }
}
