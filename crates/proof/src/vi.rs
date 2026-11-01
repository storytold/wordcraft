//! Telling Vietnamese text apart from English, so a document typed in Vietnamese under the default
//! English proofing language isn't covered in red underlines (as Word's "detect language
//! automatically" does). There are no Vietnamese proofing tools yet, so Vietnamese text is treated
//! like any language without them: no spelling or grammar marks.
//!
//! The test is orthographic. Vietnamese writes most syllables with letters no other Latin-script
//! language shares: the horn letters (`ơ ư`), `ĩ ũ`, and the tone-marked vowels of the Latin
//! Extended Additional block (`ạ ả ấ ầ … ỹ`), precomposed or as combining hook above, dot below
//! and horn. French and Portuguese share `â ê ô à é`, Romanian `ă` and Croatian, Serbian and
//! Bosnian `đ`, so those alone don't count.

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
}

#[cfg(test)]
mod checker_tests {
    use crate::{Options, ProofLang, check_text_with};

    #[test]
    fn vietnamese_paragraphs_under_english_get_no_marks() {
        let text = "Hôm nay tôi đi học, trời rất đẹp và mát mẻ.";
        let spans = [(0..text.len(), ProofLang::En)];
        assert!(check_text_with(text, &spans, &Options::default(), true, false).is_empty());
        // No language at all (the document default) behaves the same.
        assert!(check_text_with(text, &[], &Options::default(), true, false).is_empty());
    }

    #[test]
    fn english_keeps_its_marks_and_skips_vietnamese_names() {
        let text = "We met Nguyễn yesterdy at the office.";
        let issues = check_text_with(text, &[(0..text.len(), ProofLang::En)], &Options::default(), false, false);
        let flagged: Vec<&str> = issues.iter().filter_map(|i| text.get(i.start..i.end)).collect();
        assert_eq!(flagged, ["yesterdy"]);
    }
}
