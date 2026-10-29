//! Russian word division, from the rules of Russian spelling (no pattern file):
//!
//! - every part keeps a vowel, and at least two letters stay on each side;
//! - `ь`, `ъ` and `й` stay with the letter before them (`паль-то`, `вой-на`), and a syllable
//!   ends after `ъ` (`объ-ём`, `подъ-езд`);
//! - between two vowels with one consonant between them the consonant starts the next syllable
//!   (`ма-ма`, `ро-ди-на`);
//! - a cluster of consonants is divided after its first consonant (`сес-тра`, `кар-ман`,
//!   `кас-са`, `под-ход`), which also keeps the common prefixes `под-`, `раз-`, `без-` whole;
//! - two vowels side by side may be divided (`по-эт`, `ма-як`).

use super::is_cyrillic;

/// Russian vowels.
fn is_vowel(c: char) -> bool {
    matches!(c, 'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я')
}

/// Letters that never begin a line: the soft and hard signs and `й`.
fn sticks_left(c: char) -> bool {
    matches!(c, 'ь' | 'ъ' | 'й')
}

/// Break points of a lowercase Russian word (letters only): each is the index of the letter that
/// starts a new line. Empty for words with letters outside the Russian alphabet.
pub fn points(c: &[char]) -> Vec<usize> {
    let n = c.len();
    if n < 4 || !c.iter().all(|&x| is_cyrillic(x) && x.is_alphabetic()) {
        return vec![];
    }
    let vowels: Vec<usize> = (0..n).filter(|&i| c.get(i).is_some_and(|&x| is_vowel(x))).collect();
    let mut out = vec![];
    for pair in vowels.windows(2) {
        let [a, b] = pair else { continue };
        let (start, end) = (a + 1, *b);
        // The letters that must stay on the left: signs and `й` right after the vowel or after a
        // consonant of the cluster.
        let mut cut = if start == end {
            // Two vowels side by side.
            end
        } else if c.get(end - 1) == Some(&'ъ') {
            // The hard sign closes its syllable (`объ-ём`, `подъ-езд`).
            end
        } else {
            // The cluster's consonants, skipping signs that stick to the letter before.
            let consonants: Vec<usize> = (start..end).filter(|&i| c.get(i).is_some_and(|&x| !sticks_left(x))).collect();
            match consonants.as_slice() {
                // Only `й` (`май-онез`): after it.
                [] => end,
                // One consonant: it starts the next syllable (`ма-ма`, `паль-то`).
                [only] => *only,
                // Several: after the first one (`сес-тра`, `кар-ман`).
                [_, second, ..] => *second,
            }
        };
        // A sign or `й` never starts a line.
        while c.get(cut).is_some_and(|&x| sticks_left(x)) && cut < end {
            cut += 1;
        }
        // Cuts come in increasing order (each lies after the previous pair's vowel), so a repeat
        // can only be the last one: no scan of the whole list per syllable (#3657).
        if cut >= 2 && n - cut >= 2 && out.last().is_none_or(|&l| l < cut) {
            out.push(cut);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(w: &str) -> String {
        let c: Vec<char> = w.chars().collect();
        let p = points(&c);
        c.iter().enumerate().map(|(i, ch)| if p.contains(&i) { format!("-{ch}") } else { ch.to_string() }).collect()
    }

    #[test]
    fn russian_words_divide_by_the_rules() {
        assert_eq!(h("мама"), "ма-ма");
        assert_eq!(h("родина"), "ро-ди-на");
        assert_eq!(h("сестра"), "сес-тра");
        assert_eq!(h("карман"), "кар-ман");
        assert_eq!(h("касса"), "кас-са");
        assert_eq!(h("подход"), "под-ход");
        assert_eq!(h("пальто"), "паль-то");
        assert_eq!(h("объём"), "объ-ём");
        assert_eq!(h("подъезд"), "подъ-езд");
        assert_eq!(h("семья"), "се-мья");
        assert_eq!(h("вьюга"), "вью-га");
        assert_eq!(h("объявление"), "объ-яв-ле-ние");
        assert_eq!(h("война"), "вой-на");
        assert_eq!(h("чайник"), "чай-ник");
        assert_eq!(h("поэзия"), "по-э-зия");
        assert_eq!(h("маяк"), "ма-як");
        assert_eq!(h("жидкие"), "жид-кие");
        assert_eq!(h("обои"), "обои");
        assert_eq!(h("декоративная"), "де-ко-ра-тив-ная");
        assert_eq!(h("штукатурка"), "шту-ка-тур-ка");
    }

    #[test]
    fn short_words_and_other_scripts_stay_whole() {
        for w in ["дом", "ёж", "оса", "iPhone", "дом2", ""] {
            let c: Vec<char> = w.chars().collect();
            assert!(points(&c).is_empty(), "{w}");
        }
    }

    #[test]
    fn no_line_starts_with_a_sign_or_ends_with_one_letter() {
        for w in ["объявление", "подъезд", "семья", "вьюга", "майонез", "яйцо", "ауди", "аорта", "оазис", "ёлочка"]
        {
            let c: Vec<char> = w.chars().collect();
            for p in points(&c) {
                assert!(p >= 2 && c.len() - p >= 2, "{w}: {p}");
                assert!(c.get(p).is_some_and(|&x| !sticks_left(x)), "{w}: {p}");
                assert!(c.get(..p).is_some_and(|l| l.iter().any(|&x| is_vowel(x))), "{w}: {p}");
                assert!(c.get(p..).is_some_and(|r| r.iter().any(|&x| is_vowel(x))), "{w}: {p}");
            }
        }
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "[а-яё]{0,30}") {
            let c: Vec<char> = s.chars().collect();
            for p in points(&c) {
                proptest::prop_assert!(p >= 2 && p + 2 <= c.len());
            }
        }
    }
}
