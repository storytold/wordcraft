//! Document-level writing statistics: English readability estimates and time estimates.
//!
//! The syllable counter is deliberately a small English heuristic, not a pronunciation
//! dictionary. Analysis scans the input without copying the document; word-frequency storage is
//! capped by both the number and size of tracked words.

use std::collections::HashMap;
use std::fmt;

const READING_WPM: f64 = 238.0;
const SPEAKING_WPM: f64 = 150.0;
const MAX_DISTINCT_WORDS: usize = 50_000;
const MAX_TRACKED_WORD_CHARS: usize = 128;
const TOP_WORDS: usize = 10;

const STOPWORDS: &[&str] = &[
    "the", "and", "that", "this", "with", "have", "from", "they", "will", "would", "there", "their", "what", "about", "which", "when", "your",
    "were", "been", "into", "than", "then", "them", "these", "some", "could", "other", "also", "just", "like", "more", "very", "for", "are", "was",
    "but", "not", "you",
];

#[derive(Debug, Clone, PartialEq)]
pub struct WritingStats {
    pub words: usize,
    pub sentences: usize,
    pub syllables: usize,
    pub avg_words_per_sentence: f64,
    pub longest_sentence_words: usize,
    pub flesch_reading_ease: f64,
    pub flesch_kincaid_grade: f64,
    pub reading_minutes: f64,
    pub speaking_minutes: f64,
    pub top_words: Vec<(String, usize)>,
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

/// Approximate English syllables by vowel groups, with a small silent-final-e rule.
/// The result is at least one, including for empty or numeric-only tokens.
pub fn count_syllables(word: &str) -> usize {
    let mut groups = 0usize;
    let mut previous_vowel = false;
    let mut before_last = None;
    let mut before_before_last = None;
    let mut last = None;

    for c in word.chars().filter(|c| c.is_alphabetic()).flat_map(char::to_lowercase) {
        let vowel = is_vowel(c);
        if vowel && !previous_vowel {
            groups = groups.saturating_add(1);
        }
        previous_vowel = vowel;
        before_before_last = before_last;
        before_last = last;
        last = Some(c);
    }

    if groups > 1 && last == Some('e') {
        // "table" and "little" keep the final consonant + le syllable.
        let consonant_le = before_last == Some('l') && before_before_last.is_some_and(|c| !is_vowel(c));
        if !consonant_le {
            groups -= 1;
        }
    }
    groups.max(1)
}

#[derive(Default)]
struct Acc {
    words: usize,
    syllables: usize,
    sentences: usize,
    sentence_words: usize,
    longest: usize,
    freq: HashMap<String, usize>,
}

impl Acc {
    fn end_word(&mut self, word: &str) {
        if !word.chars().any(char::is_alphanumeric) {
            return;
        }

        self.words = self.words.saturating_add(1);
        self.sentence_words = self.sentence_words.saturating_add(1);
        self.syllables = self.syllables.saturating_add(count_syllables(word));

        // Frequency keys are bounded individually and in total. Long hostile tokens still count
        // as words, but are not copied into the frequency table.
        let char_count = word.chars().count();
        if !(3..=MAX_TRACKED_WORD_CHARS).contains(&char_count) {
            return;
        }
        let countable = word.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '\u{2019}');
        if !countable {
            return;
        }

        let lower = word.to_lowercase();
        if STOPWORDS.contains(&lower.as_str()) {
            return;
        }
        if let Some(n) = self.freq.get_mut(&lower) {
            *n = n.saturating_add(1);
        } else if self.freq.len() < MAX_DISTINCT_WORDS {
            self.freq.insert(lower, 1);
        }
    }

    fn end_sentence(&mut self) {
        if self.sentence_words > 0 {
            self.sentences = self.sentences.saturating_add(1);
            self.longest = self.longest.max(self.sentence_words);
            self.sentence_words = 0;
        }
    }
}

/// Analyse plain text. Empty text produces zero-valued statistics.
///
/// The scan does not allocate a character copy of the document. The repeated-word table is
/// bounded to 50,000 distinct words of at most 128 Unicode scalar values each.
pub fn analyze(text: &str) -> WritingStats {
    let mut acc = Acc::default();
    let mut word_start = None;
    let mut previous = None;

    let mut chars = text.char_indices().peekable();
    while let Some((byte_index, c)) = chars.next() {
        let next = chars.peek().map(|(_, next)| *next);
        let inner_apostrophe = (c == '\'' || c == '\u{2019}') && word_start.is_some() && next.is_some_and(char::is_alphabetic);
        let decimal_point = c == '.' && previous.is_some_and(|p: char| p.is_ascii_digit()) && next.is_some_and(|n| n.is_ascii_digit());

        if c.is_alphanumeric() || inner_apostrophe || decimal_point {
            word_start.get_or_insert(byte_index);
        } else {
            if let Some(start) = word_start.take() {
                acc.end_word(&text[start..byte_index]);
            }
            if matches!(c, '.' | '!' | '?') {
                acc.end_sentence();
            }
        }
        previous = Some(c);
    }

    if let Some(start) = word_start {
        acc.end_word(&text[start..]);
    }
    acc.end_sentence();

    let (avg, ease, grade) = if acc.words == 0 || acc.sentences == 0 {
        (0.0, 0.0, 0.0)
    } else {
        let words_per_sentence = acc.words as f64 / acc.sentences as f64;
        let syllables_per_word = acc.syllables as f64 / acc.words as f64;
        (
            words_per_sentence,
            206.835 - 1.015 * words_per_sentence - 84.6 * syllables_per_word,
            0.39 * words_per_sentence + 11.8 * syllables_per_word - 15.59,
        )
    };

    let mut top_words: Vec<(String, usize)> = acc.freq.into_iter().filter(|(_, n)| *n > 1).collect();
    top_words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top_words.truncate(TOP_WORDS);

    WritingStats {
        words: acc.words,
        sentences: acc.sentences,
        syllables: acc.syllables,
        avg_words_per_sentence: avg,
        longest_sentence_words: acc.longest,
        flesch_reading_ease: ease,
        flesch_kincaid_grade: grade,
        reading_minutes: acc.words as f64 / READING_WPM,
        speaking_minutes: acc.words as f64 / SPEAKING_WPM,
        top_words,
    }
}

/// A plain-English label for the Flesch Reading Ease score.
pub fn flesch_reading_ease_label(score: f64) -> &'static str {
    if score >= 90.0 {
        "very easy"
    } else if score >= 70.0 {
        "easy"
    } else if score >= 60.0 {
        "plain English"
    } else if score >= 50.0 {
        "fairly difficult"
    } else if score >= 30.0 {
        "difficult"
    } else {
        "very difficult"
    }
}

fn minutes_label(minutes: f64) -> String {
    let seconds = (minutes * 60.0).round().max(0.0) as u64;
    if seconds < 60 { format!("{seconds} sec") } else { format!("{} min {} sec", seconds / 60, seconds % 60) }
}

impl fmt::Display for WritingStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Words:               {}", self.words)?;
        writeln!(f, "Sentences:           {}", self.sentences)?;
        writeln!(f, "Avg words/sentence:  {:.1}", self.avg_words_per_sentence)?;
        writeln!(f, "Longest sentence:    {} words", self.longest_sentence_words)?;
        writeln!(f, "Flesch reading ease: {:.1} ({})", self.flesch_reading_ease, flesch_reading_ease_label(self.flesch_reading_ease))?;
        writeln!(f, "Grade level:         {:.1}", self.flesch_kincaid_grade)?;
        writeln!(f, "Reading time:        {}", minutes_label(self.reading_minutes))?;
        writeln!(f, "Speaking time:       {}", minutes_label(self.speaking_minutes))?;
        if !self.top_words.is_empty() {
            let list: Vec<String> = self.top_words.iter().map(|(word, count)| format!("{word} ({count})")).collect();
            writeln!(f, "Most used words:     {}", list.join(", "))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syllables() {
        assert_eq!(count_syllables("cat"), 1);
        assert_eq!(count_syllables("make"), 1);
        assert_eq!(count_syllables("table"), 2);
        assert_eq!(count_syllables("little"), 2);
        assert_eq!(count_syllables("beautiful"), 3);
        assert_eq!(count_syllables("the"), 1);
        assert_eq!(count_syllables(""), 1);
        assert_eq!(count_syllables("123"), 1);
    }

    #[test]
    fn basic_counts() {
        let stats = analyze("The cat sat on the mat. Did it move? Yes!");
        assert_eq!(stats.words, 10);
        assert_eq!(stats.sentences, 3);
        assert_eq!(stats.longest_sentence_words, 6);
    }

    #[test]
    fn empty_and_whitespace_are_all_zero() {
        for text in ["", "   \n\t ", "...", "?!"] {
            let stats = analyze(text);
            assert_eq!(stats.words, 0);
            assert_eq!(stats.sentences, 0);
            assert_eq!(stats.flesch_reading_ease, 0.0);
            assert_eq!(stats.reading_minutes, 0.0);
        }
    }

    #[test]
    fn text_without_final_punctuation_counts_a_sentence() {
        assert_eq!(analyze("no full stop here").sentences, 1);
    }

    #[test]
    fn decimals_and_contractions_stay_whole() {
        let stats = analyze("It costs 3.14 dollars and isn't cheap.");
        assert_eq!(stats.sentences, 1);
        assert_eq!(stats.words, 7);
    }

    #[test]
    fn repeated_words_are_ranked_by_count_then_alphabetically() {
        let stats = analyze("Goose goose goose. Quill quill. Paper paper.");
        assert_eq!(stats.top_words.first(), Some(&("goose".to_string(), 3)));
        assert_eq!(stats.top_words.get(1), Some(&("paper".to_string(), 2)));
        assert_eq!(stats.top_words.get(2), Some(&("quill".to_string(), 2)));
    }

    #[test]
    fn hostile_text_does_not_panic_or_store_unbounded_tokens() {
        let long = "a".repeat(1_000_000);
        let stats = analyze(&long);
        assert_eq!(stats.words, 1);
        assert!(stats.top_words.len() <= TOP_WORDS);

        let _ = analyze("\u{0}\u{FFFF}\u{1F980} ... ''' 1.2.3 ’’’ é");
        let many: String = (0..60_000).map(|i| format!("w{i}x ")).collect();
        let stats = analyze(&many);
        assert_eq!(stats.words, 60_000);
        assert!(stats.top_words.len() <= TOP_WORDS);
    }

    #[test]
    fn labels() {
        assert_eq!(minutes_label(0.0), "0 sec");
        assert_eq!(minutes_label(1.5), "1 min 30 sec");
        assert_eq!(flesch_reading_ease_label(65.0), "plain English");
    }
}
