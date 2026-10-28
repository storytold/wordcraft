//! Thai proofing from the word list of `kham-core` (CC0, compiled into that crate: nothing is
//! installed or bundled by WordCraft). Thai has no spaces between words, so a run of Thai letters
//! is segmented first; a piece the list doesn't know is a possible misspelling. Suggestions come
//! from the same crate's spell checker. The segmentation also gives the places where a line may
//! break inside a Thai paragraph (UAX #14 finds none), and Thai is never hyphenated.

use std::ops::Range;
use std::sync::OnceLock;

use kham_core::spell::SpellChecker;
use kham_core::{Token, TokenKind, Tokenizer};

/// The word segmenter, built on first use (about 70 ms).
static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();
/// The spell checker, built on first use (about 130 ms): only a suggestion asks for it.
static SPELLER: OnceLock<SpellChecker> = OnceLock::new();

fn tokenizer() -> &'static Tokenizer {
    TOKENIZER.get_or_init(Tokenizer::new)
}

fn speller() -> &'static SpellChecker {
    SPELLER.get_or_init(SpellChecker::builtin)
}

/// Build the segmenter now, so the first Thai paragraph isn't slowed by it. Cheap when done.
pub fn warm_up() {
    let _ = tokenizer();
}

/// Is `c` in the Thai block (letters, vowel signs, tone marks, digits, punctuation)?
pub fn is_thai_char(c: char) -> bool {
    ('\u{0E00}'..='\u{0E7F}').contains(&c)
}

/// Does `text` have a Thai character?
pub fn has_thai(text: &str) -> bool {
    text.chars().any(is_thai_char)
}

/// A piece of Thai text the word list doesn't know.
fn is_unknown_thai(t: &Token<'_>) -> bool {
    t.kind == TokenKind::Unknown && has_thai(t.text)
}

/// A known Thai piece short enough to be the rest of a misspelled word (`วัด` and `ดี` after the
/// stray `ส` of `สวัดดี`), not the start of the next word.
fn is_short_thai(t: &Token<'_>) -> bool {
    t.kind == TokenKind::Thai && t.char_len() <= 3
}

/// How many known short pieces following an unknown one belong to the misspelled word.
const MAX_TAIL: usize = 2;

/// The byte ranges of `run` that are possible misspellings: pieces the word list doesn't know,
/// joined with the short pieces right after them (`สวัดดี` segments as `ส` `วัด` `ดี`, and one
/// mark under the whole word is what the writer expects). Latin letters, numbers and punctuation
/// in the run are not judged. Ranges are sorted, disjoint, and on character boundaries.
pub fn misspelled_ranges(run: &str) -> Vec<Range<usize>> {
    if !has_thai(run) {
        return Vec::new();
    }
    let tokens = tokenizer().segment(run);
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while let Some(first) = tokens.get(i) {
        if !is_unknown_thai(first) {
            i += 1;
            continue;
        }
        let start = first.span.start;
        let mut end = first.span.end;
        let mut tail = 0;
        let mut j = i + 1;
        while let Some(next) = tokens.get(j) {
            if next.span.start != end {
                break;
            }
            if is_unknown_thai(next) {
                tail = 0;
            } else if is_short_thai(next) && tail < MAX_TAIL {
                tail += 1;
            } else {
                break;
            }
            end = next.span.end;
            j += 1;
        }
        // A tail that ends on an unknown piece is fine; one that ends on known ones is kept too.
        match out.last_mut() {
            Some(last) if start <= last.end => last.end = last.end.max(end),
            _ => out.push(start..end),
        }
        i = j;
    }
    out
}

/// Is `word` free of unknown Thai pieces? A word without Thai letters isn't judged here.
pub fn is_correct(word: &str) -> bool {
    misspelled_ranges(word).is_empty()
}

/// Up to `max` corrections for the misspelled `word`, best first.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    if max == 0 || !has_thai(word) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for s in speller().suggestions(word, max.saturating_add(1)) {
        // The word itself isn't a correction of it.
        if s.word != word && !out.contains(&s.word) {
            out.push(s.word.to_string());
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Characters a line may not end with: a leading vowel (`เ แ โ ใ ไ`) belongs to the consonant
/// after it.
fn is_leading_vowel(c: char) -> bool {
    matches!(c, '\u{0E40}'..='\u{0E44}')
}

/// Characters a line may not start with: the repetition mark `ๆ`, the abbreviation mark `ฯ`, and
/// the other marks that follow the letter before them.
fn cannot_start_line(c: char) -> bool {
    matches!(c, '\u{0E2F}' | '\u{0E46}' | '\u{0E4F}' | '\u{0E5A}' | '\u{0E5B}')
        || ('\u{0E30}'..='\u{0E3A}').contains(&c)
        || ('\u{0E47}'..='\u{0E4E}').contains(&c)
}

/// A Thai digit (`๐`–`๙`).
fn is_thai_digit(c: char) -> bool {
    ('\u{0E50}'..='\u{0E59}').contains(&c)
}

/// Would a break between `before` and `after` split an amount: after the baht sign `฿`, before it
/// when a Thai digit comes first (`๑๒๓฿`), or between two Thai digits?
fn splits_amount(before: char, after: char) -> bool {
    before == '\u{0E3F}' || (is_thai_digit(before) && (is_thai_digit(after) || after == '\u{0E3F}'))
}

/// Byte offsets of `text` where a line may break between two Thai words, found by segmenting it
/// with the word list. Sorted; never 0 or `text.len()`; always on a character boundary; only
/// between two Thai characters (UAX #14 decides everywhere else), never after a leading vowel and
/// never before a character that can't start a line.
pub fn break_offsets(text: &str) -> Vec<usize> {
    if !has_thai(text) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for t in tokenizer().segment(text) {
        let at = t.span.start;
        if at == 0 || at >= text.len() {
            continue;
        }
        let (Some(before), Some(after)) = (text.get(..at).and_then(|s| s.chars().next_back()), t.text.chars().next()) else { continue };
        if is_thai_char(before) && is_thai_char(after) && !is_leading_vowel(before) && !cannot_start_line(after) && !splits_amount(before, after) {
            out.push(at);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad(run: &str) -> Vec<&str> {
        misspelled_ranges(run).into_iter().filter_map(|r| run.get(r)).collect()
    }

    fn pieces(text: &str) -> Vec<&str> {
        let mut at = 0;
        let mut out = Vec::new();
        for b in break_offsets(text).into_iter().chain([text.len()]) {
            if let Some(p) = text.get(at..b) {
                out.push(p);
            }
            at = b;
        }
        out
    }

    #[test]
    fn correct_words_and_sentences_are_not_flagged() {
        for w in ["สวัสดี", "ควร", "กินข้าว", "กินข้าวกับปลา", "ฉันไปโรงเรียนวันนี้", "ครับ"]
        {
            assert!(bad(w).is_empty(), "{w}: {:?}", bad(w));
            assert!(is_correct(w), "{w}");
        }
    }

    #[test]
    fn a_misspelled_word_is_one_mark() {
        assert_eq!(bad("สวัดดี"), vec!["สวัดดี"]);
        assert!(!is_correct("สวัดดี"));
        // Inside a sentence, only the word, not its neighbours.
        assert_eq!(bad("ฉันไปโรงเรียนวันนี้สวัดดีครับ"), vec!["สวัดดี"]);
        assert_eq!(bad("พิมพ์ผิดตรงนี้ฟฟฟฟ"), vec!["ฟฟฟฟ"]);
    }

    #[test]
    fn latin_numbers_and_punctuation_are_not_judged() {
        for w in ["Hello", "123", "", "...", "Hello สวัสดี 123"] {
            assert!(bad(w).is_empty(), "{w}");
        }
        assert!(is_correct("Hello"));
    }

    #[test]
    fn ranges_are_sorted_disjoint_and_on_boundaries() {
        let text = "สวัดดีครับ ฉันกินข้าวฟฟฟฟ และ Hello สวัดดี";
        let ranges = misspelled_ranges(text);
        assert!(ranges.len() >= 2, "{ranges:?}");
        let mut end = 0;
        for r in ranges {
            assert!(r.start >= end && r.start < r.end, "{r:?}");
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end), "{r:?}");
            end = r.end;
        }
    }

    #[test]
    fn suggestions_put_the_right_word_first() {
        assert_eq!(suggest("สวัดดี", 5).first().map(String::as_str), Some("สวัสดี"));
        assert!(suggest("สวัดดี", 3).len() <= 3);
        assert!(suggest("สวัดดี", 0).is_empty());
        assert!(suggest("hello", 5).is_empty());
        assert!(suggest("", 5).is_empty());
    }

    #[test]
    fn lines_break_between_thai_words() {
        let text = "ฉันไปโรงเรียนวันนี้";
        assert_eq!(pieces(text), vec!["ฉัน", "ไป", "โรงเรียน", "วันนี้"]);
        // Never at the ends, never inside Latin text or between Latin and Thai.
        assert!(break_offsets("Hello").is_empty());
        assert!(break_offsets("Helloสวัสดี").is_empty());
        assert!(break_offsets("").is_empty());
        assert!(break_offsets("ก").is_empty());
    }

    #[test]
    fn a_leading_vowel_stays_with_its_consonant_and_marks_stay_with_their_word() {
        for text in ["เด็กไปโรงเรียนแล้วโดยไม่ได้ใช้ไม้ยมกซ้ำๆ", "เขาไปกรุงเทพฯ แล้ว", "ไปๆมาๆ"]
        {
            for b in break_offsets(text) {
                let before = text.get(..b).and_then(|s| s.chars().next_back());
                let after = text.get(b..).and_then(|s| s.chars().next());
                assert!(before.is_some_and(|c| !is_leading_vowel(c)), "{text}: after a leading vowel at {b}");
                assert!(after.is_some_and(|c| !cannot_start_line(c)), "{text}: before {after:?} at {b}");
            }
        }
    }

    #[test]
    fn a_baht_sign_and_thai_digits_stay_together() {
        for text in ["฿๑๒๓", "ราคา ฿๑๒๓ บาท", "ราคา฿๑๒๓", "๑๒๓๔๕", "๑๒๓฿๔๕๖"] {
            for b in break_offsets(text) {
                let before = text.get(..b).and_then(|s| s.chars().next_back());
                let after = text.get(b..).and_then(|s| s.chars().next());
                let (Some(before), Some(after)) = (before, after) else { continue };
                assert!(!splits_amount(before, after), "{text}: break at {b} splits an amount");
            }
        }
        // Independent of `splits_amount`: an amount stays whole, in either order.
        for amount in ["฿๑๒๓", "๑๒๓฿", "฿๑๒๓๔๕๖", "๑๒๓๔๕๖฿"] {
            assert!(break_offsets(amount).is_empty(), "{amount}: {:?}", break_offsets(amount));
        }
    }

    #[test]
    fn hostile_text_never_panics() {
        let long = "ก".repeat(20_000);
        let _ = break_offsets(&long);
        let _ = misspelled_ranges(&long);
        for s in ["\u{0E31}", "\u{0E48}\u{0E48}\u{0E48}", "เ", "ๆ", "\u{0E3A}ก", "ก\u{200B}ข", "ก\u{0E31}\u{0E31}\u{0E31}ข", "\u{FFFD}ก"]
        {
            let _ = break_offsets(s);
            let _ = misspelled_ranges(s);
            let _ = suggest(s, 3);
        }
    }
}
