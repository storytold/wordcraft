//! Turkish spelling suggestions, ranked by likelihood.
//!
//! Candidates are generated and ranked by cost:
//! 1. Capitalization / case adjustments (especially dotted/dotless I: `istanbul` -> `İstanbul`).
//! 2. Missing Turkish diacritics / ASCII substitutions (most frequent Turkish typing slip):
//!    `c` <-> `ç`, `g` <-> `ğ`, `i` <-> `ı`, `o` <-> `ö`, `s` <-> `ş`, `u` <-> `ü` (`turkce` -> `Türkçe`).
//! 3. Common spelling mistakes and phonetic confusions (`REP` table):
//!    `yalnış` -> `yanlış`, `yanlız` -> `yalnız`, `herşey` -> `her şey`, `bir kaç` -> `birkaç`.
//! 4. Missing space for detached particles and conjunctions (`bende` -> `ben de`, `geldinmi` -> `geldin mi`).
//! 5. Single typo (Damerau-Levenshtein distance <= 2).

use std::collections::HashSet;

use crate::tr::dict::TrDict;

/// Common misspellings and their canonical replacements in Turkish.
const COMMON_REPLACEMENTS: &[(&str, &str)] = &[
    ("yalnış", "yanlış"),
    ("yanlız", "yalnız"),
    ("herşey", "her şey"),
    ("herkez", "herkes"),
    ("bir kaç", "birkaç"),
    ("birşey", "bir şey"),
    ("tabiki", "tabii ki"),
    ("orjinal", "orijinal"),
    ("kiprik", "kirpik"),
    ("şöför", "şoför"),
    ("sarmısak", "sarımsak"),
    ("makina", "makine"),
    ("ünvan", "unvan"),
    ("rastlantı", "rastlantı"),
    ("labaratuvar", "laboratuvar"),
    ("laboratuar", "laboratuvar"),
    ("kravat", "kravat"),
    ("gardolap", "gardırop"),
    ("egzoz", "egzoz"),
    ("egzozt", "egzoz"),
    ("egsoz", "egzoz"),
    ("gelcen", "geleceksin"),
    ("gelecen", "geleceksin"),
    ("gitcen", "gideceksin"),
    ("gidecen", "gideceksin"),
    ("yapcan", "yapacaksın"),
    ("yapacan", "yapacaksın"),
    ("edecen", "edeceksin"),
    ("deyil", "değil"),
];

/// Longest word (in letters) given suggestions. Every candidate is a fresh copy of the word, so
/// the cost grows with the square of its length (#3625); Turkish words run longer than most
/// (`Çekoslovakyalılaştıramadıklarımızdanmışsınız` is 43 letters), so the cap is above the other
/// languages' 40.
const MAX_LETTERS: usize = 64;

/// Produce spelling suggestions for a misspelled Turkish word.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let dict = super::dictionary();
    suggest_with(&dict, word, max)
}

/// Suggest with a given dictionary.
pub fn suggest_with(d: &TrDict, word: &str, max: usize) -> Vec<String> {
    let n = word.chars().count();
    if n == 0 || n > MAX_LETTERS || max == 0 {
        return Vec::new();
    }

    let mut suggestions = Vec::new();
    let mut seen = HashSet::new();

    let lower = turkish_to_lower(word);

    // 1. Check direct common replacement table
    for &(wrong, right) in COMMON_REPLACEMENTS {
        if lower == wrong && seen.insert(right.to_string()) {
            suggestions.push(right.to_string());
            if suggestions.len() >= max {
                return suggestions;
            }
        }
    }
    // Only the table takes more than one word (`bir kaç`).
    if word.contains(char::is_whitespace) {
        return suggestions;
    }

    // 2. Diacritic substitutions (`c` -> `ç`, `g` -> `ğ`, `ı` -> `i`, `o` -> `ö`, `s` -> `ş`, `u` -> `ü`)
    if !d.is_empty() {
        let variants = generate_diacritic_variants(&lower);
        for cand in variants {
            if cand != lower && d.contains(&cand) && seen.insert(cand.clone()) {
                suggestions.push(match_casing(word, &cand));
                if suggestions.len() >= max {
                    return suggestions;
                }
            }
        }
    }

    // 3. Detached suffix check: `-de`, `-da`, `-mi`, `-mı`, `-mu`, `-mü`
    for suffix in ["de", "da", "te", "ta", "mi", "mı", "mu", "mü"] {
        if lower.ends_with(suffix) && lower.len() > suffix.len() + 1 {
            let stem = &lower[..lower.len() - suffix.len()];
            if d.is_empty() || d.contains(stem) {
                let cand = format!("{stem} {suffix}");
                if seen.insert(cand.clone()) {
                    suggestions.push(match_casing(word, &cand));
                    if suggestions.len() >= max {
                        return suggestions;
                    }
                }
            }
        }
    }

    // 4. Single typo (transposition, deletion, insertion, substitution)
    if !d.is_empty() {
        let chars: Vec<char> = lower.chars().collect();
        let n = chars.len();

        // 4a. Transposition (adjacent swap: `sorna` -> `sonra`)
        for i in 0..n.saturating_sub(1) {
            if chars.get(i) != chars.get(i + 1) {
                let mut swapped = chars.clone();
                swapped.swap(i, i + 1);
                let cand: String = swapped.into_iter().collect();
                if d.contains(&cand) && seen.insert(cand.clone()) {
                    suggestions.push(match_casing(word, &cand));
                    if suggestions.len() >= max {
                        return suggestions;
                    }
                }
            }
        }

        // Turkish alphabet for insertions and substitutions
        const TR_ALPHABET: &[char] = &[
            'a', 'b', 'c', 'ç', 'd', 'e', 'f', 'g', 'ğ', 'h', 'ı', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'ö', 'p', 'r', 's', 'ş', 't', 'u', 'ü', 'v',
            'y', 'z',
        ];

        // 4b. Deletion (one character left out: `kitapp` -> `kitap`)
        for i in 0..n {
            let mut deleted = chars.clone();
            deleted.remove(i);
            let cand: String = deleted.into_iter().collect();
            if d.contains(&cand) && seen.insert(cand.clone()) {
                suggestions.push(match_casing(word, &cand));
                if suggestions.len() >= max {
                    return suggestions;
                }
            }
        }

        // 4c. Substitution (one wrong letter: `yolculuk` -> ...)
        for i in 0..n {
            for &c in TR_ALPHABET {
                if chars.get(i) != Some(&c) {
                    let mut substituted = chars.clone();
                    if let Some(target) = substituted.get_mut(i) {
                        *target = c;
                    }
                    let cand: String = substituted.into_iter().collect();
                    if d.contains(&cand) && seen.insert(cand.clone()) {
                        suggestions.push(match_casing(word, &cand));
                        if suggestions.len() >= max {
                            return suggestions;
                        }
                    }
                }
            }
        }

        // 4d. Insertion (one missing letter: `kitp` -> `kitap`)
        for i in 0..=n {
            for &c in TR_ALPHABET {
                let mut inserted = chars.clone();
                inserted.insert(i, c);
                let cand: String = inserted.into_iter().collect();
                if d.contains(&cand) && seen.insert(cand.clone()) {
                    suggestions.push(match_casing(word, &cand));
                    if suggestions.len() >= max {
                        return suggestions;
                    }
                }
            }
        }
    }

    // 5. Fallback: capitalization
    let cap = super::grammar::turkish_capitalize(&lower);
    if cap != word && (d.is_empty() || d.contains(&cap)) && seen.insert(cap.clone()) {
        suggestions.push(cap);
    }

    suggestions
}

/// Convert ASCII letters to Turkish diacritic equivalents.
fn generate_diacritic_variants(s: &str) -> Vec<String> {
    let mut results = vec![String::new()];
    for c in s.chars() {
        let (alt1, alt2) = match c {
            'c' => ('c', Some('ç')),
            'ç' => ('ç', Some('c')),
            'g' => ('g', Some('ğ')),
            'ğ' => ('ğ', Some('g')),
            'i' => ('i', Some('ı')),
            'ı' => ('ı', Some('i')),
            'o' => ('o', Some('ö')),
            'ö' => ('ö', Some('o')),
            's' => ('s', Some('ş')),
            'ş' => ('ş', Some('s')),
            'u' => ('u', Some('ü')),
            'ü' => ('ü', Some('u')),
            other => (other, None),
        };

        let branch_mult = if alt2.is_some() { 2 } else { 1 };
        if results.len() * branch_mult > 1024 {
            // Cap search branch explosion
            for res in &mut results {
                res.push(c);
            }
            continue;
        }

        let mut next_gen = Vec::with_capacity(results.len() * branch_mult);
        for existing in &results {
            let mut s1 = existing.clone();
            s1.push(alt1);
            next_gen.push(s1);
            if let Some(c2) = alt2 {
                let mut s2 = existing.clone();
                s2.push(c2);
                next_gen.push(s2);
            }
        }
        results = next_gen;
    }
    results
}

/// Turkish lowercase with dotted/dotless I support.
pub fn turkish_to_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'I' => out.push('ı'),
            'İ' => out.push('i'),
            other => {
                for lc in other.to_lowercase() {
                    out.push(lc);
                }
            }
        }
    }
    out
}

/// Match original casing on suggestion.
fn match_casing(orig: &str, cand: &str) -> String {
    if orig.chars().all(char::is_uppercase) {
        cand.to_uppercase()
    } else if orig.chars().next().is_some_and(char::is_uppercase) {
        super::grammar::turkish_capitalize(cand)
    } else {
        cand.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_replacements_are_suggested() {
        let sugs = suggest("yalnış", 3);
        assert!(sugs.contains(&"yanlış".to_string()));

        let sugs2 = suggest("herkez", 3);
        assert!(sugs2.contains(&"herkes".to_string()));
    }

    #[test]
    fn detached_suffix_is_suggested() {
        let sugs = suggest("geldinmi", 3);
        assert!(sugs.contains(&"geldin mi".to_string()));
    }

    #[test]
    fn overlong_words_get_no_suggestions_promptly() {
        let d = TrDict::from_hunspell("SET UTF-8\nSFX 1 Y 1\nSFX 1 0 lar .\n", "2\nkitap/1\nsonra\n").unwrap();
        assert!(suggest_with(&d, "sorna", 3).contains(&"sonra".to_string()));
        assert!(suggest_with(&d, "kitaplr", 3).contains(&"kitaplar".to_string()));
        let t = std::time::Instant::now();
        let long = "a".repeat(100_000);
        assert!(suggest_with(&d, &long, 8).is_empty());
        assert!(!d.contains(&long) && !d.contains(&format!("kitap{long}")));
        assert!(!super::super::is_correct_with(&d, &long));
        // Tens of milliseconds with the caps; without them this ran for over 5 minutes.
        assert!(t.elapsed().as_secs_f64() < 10.0, "{:?}", t.elapsed());
    }

    #[test]
    fn two_words_get_only_the_tables_replacement() {
        let d = TrDict::from_hunspell("SET UTF-8\n", "2\nbirkaç\nsonra\n").unwrap();
        assert_eq!(suggest_with(&d, "bir kaç", 3), vec!["birkaç".to_string()]);
        assert_eq!(suggest_with(&d, "Bir kaç", 3), vec!["birkaç".to_string()]);
        assert!(suggest_with(&d, "sorna sorna", 3).is_empty());
    }

    #[test]
    fn turkish_to_lower_handles_i() {
        assert_eq!(turkish_to_lower("IŞIK"), "ışık");
        assert_eq!(turkish_to_lower("İSTANBUL"), "istanbul");
    }

    #[test]
    fn typo_transposition_is_suggested() {
        // Suggestions come from the installed Turkish dictionary; without one there are none.
        if !super::super::has_dictionary() {
            return;
        }
        let sugs = suggest("sorna", 3);
        assert!(sugs.contains(&"sonra".to_string()), "sorna should suggest sonra, got {sugs:?}");

        let sugs_gelcen = suggest("gelcen", 3);
        assert!(sugs_gelcen.contains(&"geleceksin".to_string()), "gelcen should suggest geleceksin, got {sugs_gelcen:?}");
    }
}
