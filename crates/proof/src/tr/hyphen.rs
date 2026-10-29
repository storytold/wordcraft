//! Turkish hyphenation by rule ([`ProofLang::Tr`](crate::ProofLang::Tr)).
//!
//! Turkish words are split at syllables according to Turkish Language Association (TDK) rules.
//! Because Turkish syllable boundaries are fully regular, they can be determined directly by rule
//! without needing external pattern files or word lists:
//! - Turkish vowels: `a`, `e`, `ı`, `i`, `o`, `ö`, `u`, `ü` (both lowercase and uppercase).
//! - A single consonant between two vowels joins the following syllable (`ba-lık`, `ö-ğren-ci`).
//! - Of two consonants between two vowels, the first stays with the preceding syllable and the
//!   second joins the next (`alt-lık`, `sev-gi`, `tür-kçe`).
//! - Of three consonants between two vowels, the first two stay with the preceding syllable and the
//!   third joins the next (`türk-çe`, `sürp-riz`).
//! - In compound words, syllable division follows pronunciation across the boundary (`ba-şöğ-ret-men`,
//!   `il-ko-kul`), which matches the general vowel/consonant distribution rules.
//!
//! A single letter left at the end or start of a line is ruled out by the paragraph's
//! [`Limits`](crate::hyphen::Limits).

/// Is `c` a Turkish vowel?
pub fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'ı' | 'i' | 'o' | 'ö' | 'u' | 'ü' | 'A' | 'E' | 'I' | 'İ' | 'O' | 'Ö' | 'U' | 'Ü')
}

/// Is `c` a Turkish letter?
pub(crate) fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

/// Break points of a Turkish word (char indices; a hyphen goes before that char).
/// Empty for words with fewer than two vowels or containing non-alphabetic characters.
pub fn points(word: &[char]) -> Vec<usize> {
    if word.len() < 2 || !word.iter().all(|&c| is_letter(c)) {
        return Vec::new();
    }
    let vowels: Vec<usize> = (0..word.len()).filter(|&i| is_vowel(word[i])).collect();
    if vowels.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for pair in vowels.windows(2) {
        let (v1, v2) = (pair[0], pair[1]);
        if v2 <= v1 {
            continue;
        }
        let num_consonants = v2 - v1 - 1;
        let pt = match num_consonants {
            0 => v1 + 1,
            1 => v2 - 1,
            2 => v1 + 2,
            _ => v2 - 1,
        };
        if pt > v1 && pt <= v2 {
            out.push(pt);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(w: &str) -> String {
        let chars: Vec<char> = w.chars().collect();
        let pts = points(&chars);
        let mut out = String::new();
        for (i, c) in chars.iter().enumerate() {
            if pts.contains(&i) {
                out.push('-');
            }
            out.push(*c);
        }
        out
    }

    #[test]
    fn syllables_follow_turkish_rules() {
        assert_eq!(split("balık"), "ba-lık");
        assert_eq!(split("araba"), "a-ra-ba");
        assert_eq!(split("öğrenci"), "öğ-ren-ci");
        assert_eq!(split("altlık"), "alt-lık");
        assert_eq!(split("sevgi"), "sev-gi");
        assert_eq!(split("türkçe"), "türk-çe");
        assert_eq!(split("saat"), "sa-at");
        assert_eq!(split("şiir"), "şi-ir");
        assert_eq!(split("ilkokul"), "il-ko-kul");
        assert_eq!(split("başöğretmen"), "ba-şöğ-ret-men");
    }

    #[test]
    fn short_or_non_words_stay_unsplit() {
        assert_eq!(split(""), "");
        assert_eq!(split("a"), "a");
        assert_eq!(split("ve"), "ve");
        assert_eq!(split("Türk"), "Türk");
        assert_eq!(split("123"), "123");
        assert_eq!(split("araba-ev"), "araba-ev");
    }
}
