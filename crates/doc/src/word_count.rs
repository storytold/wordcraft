//! Document statistics use Khmer word boundaries independently of justification spaces.
use crate::para;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Words in text: Khmer model/dictionary spans when available, whitespace tokens
/// for other scripts, and one word per Han ideograph or kana. This does not edit
/// text or create justification spaces. Without Khmer data, retain spaced counts.
pub fn count_words(text: &str) -> usize {
    if !text.chars().any(|c| ('\u{1780}'..='\u{17ff}').contains(&c)) {
        return spaced_words(text);
    }
    if !wordcraft_proof::khmer::data_available() {
        return text.split('\u{200b}').fold(0usize, |n, part| n.saturating_add(spaced_words(part)));
    }
    // Each paragraph gets the same refinement budget as line breaking. Joining
    // paragraphs in a selection must not spend one shared budget across a document.
    text.split(['\n', '\r']).fold(0usize, |total, para| total.saturating_add(khmer_words(para)))
}

// Counts recur on every document edit and in multiple statistics surfaces. Cache
// complete paragraph text, so changes to public fields cannot leave a stale count.
// Bound retained text and entry count; unusually large paragraphs are not cached.
// Saturation stops admission rather than evicting existing paragraphs: sequential
// scans of a document larger than the cache must retain the hits that fit. This
// bounded memo keeps those entries until process exit; it is not an LRU cache.
const MAX_TEXT: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 1 << 18;
const MAX_PARAGRAPH: usize = 64 * 1024;
#[derive(Default)]
struct Counts {
    words: HashMap<String, usize>,
    bytes: usize,
}
impl Counts {
    fn remember(&mut self, text: &str, words: usize) {
        self.remember_with_limits(text, words, MAX_ENTRIES, MAX_TEXT);
    }
    fn remember_with_limits(&mut self, text: &str, words: usize, max_entries: usize, max_text: usize) {
        if self.words.contains_key(text) || self.words.len() >= max_entries || self.bytes.saturating_add(text.len()) > max_text {
            return;
        }
        self.bytes = self.bytes.saturating_add(text.len());
        self.words.insert(text.to_owned(), words);
    }
}
fn counts() -> &'static Mutex<Counts> {
    static COUNTS: OnceLock<Mutex<Counts>> = OnceLock::new();
    COUNTS.get_or_init(Mutex::default)
}
fn khmer_words(text: &str) -> usize {
    if !text.chars().any(|c| ('\u{1780}'..='\u{17ff}').contains(&c)) {
        return spaced_words(text);
    }
    if let Some(words) = counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner).words.get(text).copied() {
        return words;
    }
    // Do the model work outside the lock so exporting cannot block an editor's
    // cache hits for the duration of segmentation.
    let words = segmented_words(text);
    if text.len() <= MAX_PARAGRAPH {
        counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner).remember(text, words);
    }
    words
}
#[cfg(test)]
thread_local! {
    // Per-thread instrumentation isolates cache reuse assertions from parallel tests.
    static SEGMENTATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn segmented_words(text: &str) -> usize {
    #[cfg(test)]
    SEGMENTATIONS.with(|n| n.set(n.get().saturating_add(1)));
    let mut words = 0usize;
    let mut end = 0;
    for range in wordcraft_proof::khmer::word_ranges(text) {
        words = words.saturating_add(text.get(end..range.start).map(spaced_words).unwrap_or(0));
        // Detached marks and a standalone repetition sign are not words. Attached
        // signs stay within their preceding word's span.
        let has_letter = text.get(range.clone()).is_some_and(|s| s.chars().any(|c| ('\u{1780}'..='\u{17b3}').contains(&c)));
        words = words.saturating_add(usize::from(has_letter));
        end = range.end;
    }
    words.saturating_add(text.get(end..).map(spaced_words).unwrap_or(0))
}

/// Words in `s`, as Word counts them: tokens separated by whitespace that contain at least one
/// alphanumeric char, plus each Han ideograph and kana as a word of its own (Chinese and Japanese
/// are written without spaces, #4547; Korean is spaced, so Hangul counts by tokens).
fn spaced_words(s: &str) -> usize {
    // The half-width voiced sound marks belong to the kana before them, not a word of their own.
    // Unicode counts Khmer vowel signs as alphabetic; alone, they are not a word either.
    let khmer_sign = |c: char| ('\u{1780}'..='\u{17ff}').contains(&c) && !('\u{1780}'..='\u{17b3}').contains(&c) && !c.is_numeric();
    let word_char = |c: char| c.is_alphanumeric() && !matches!(c, '\u{FF9E}' | '\u{FF9F}') && !khmer_sign(c);
    let tokens = s.split(|c: char| c.is_whitespace() || c == para::OBJ || is_cjk_word(c)).filter(|w| w.chars().any(word_char)).count();
    tokens.saturating_add(s.chars().filter(|c| is_cjk_word(*c)).count())
}

/// A Han ideograph or kana letter (not CJK punctuation), which Word counts as one word.
fn is_cjk_word(c: char) -> bool {
    matches!(c,
        '\u{3005}'..='\u{3007}' // 々 〆 〇
        | '\u{3031}'..='\u{3035}' | '\u{303B}' // vertical kana repeat marks, 〻
        | '\u{3040}'..='\u{30FF}' // hiragana, katakana
        | '\u{31F0}'..='\u{31FF}' // katakana phonetic extensions
        | '\u{3400}'..='\u{4DBF}' // extension A
        | '\u{4E00}'..='\u{9FFF}' // unified ideographs
        | '\u{F900}'..='\u{FAFF}' // compatibility ideographs
        | '\u{FF66}'..='\u{FF9D}' // half-width katakana
        | '\u{1B000}'..='\u{1B16F}' // kana supplement and extended
        | '\u{20000}'..='\u{3FFFF}' // extensions B and later
    ) && c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;
    const PHRASE: &str = "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ";
    #[test]
    fn khmer_words_follow_shared_spans_and_manual_breaks() {
        let available = wordcraft_proof::khmer::data_available();
        assert_eq!(count_words(PHRASE), if available { 4 } else { 1 });
        assert_eq!(count_words("ខ្មែរ\u{200b}ភាសា"), 2, "authored breaks work even without a dictionary");
        assert_eq!(count_words("ៗ។! ា្"), 0, "detached marks/signs are not words");
        assert_eq!(count_words("១២៣"), 1, "Khmer digits are");
        if available {
            // A normal word, a compound, and the repetition sign's preceding word.
            assert_eq!(count_words("អ្នកចម្រៀង ក្រុមហ៊ុន អ្វីៗ"), 2 + 1 + 1);
            let words = wordcraft_proof::khmer::line_breaks(PHRASE).len() + 1;
            assert_eq!(count_words(PHRASE), words);
        }
    }
    /// A document many times larger than a long book keeps every paragraph for its next count.
    #[test]
    fn a_large_document_keeps_its_paragraphs_cached_between_counts() {
        let mut cache = Counts::default();
        let paragraphs: Vec<String> = (0..20_000).map(|n| format!("ខ្មែរ {n}")).collect();
        for _ in 0..2 {
            for paragraph in &paragraphs {
                cache.remember(paragraph, 2);
            }
        }
        assert_eq!(cache.words.len(), paragraphs.len(), "no entry was cleared before its next use");
        assert!(paragraphs.iter().all(|paragraph| cache.words.contains_key(paragraph)));
        let bytes: usize = paragraphs.iter().map(String::len).sum();
        assert_eq!(cache.bytes, bytes, "a repeated paragraph is not charged twice");
    }
    #[test]
    fn saturated_cache_preserves_reuse_across_scans_and_bounds_memory() {
        // Exceed both budgets independently without allocating production-sized caches.
        for (max_entries, max_text) in [(64, usize::MAX), (usize::MAX, 256)] {
            let paragraphs: Vec<_> = (0..256).map(|n| format!("ខ្មែរ {n}")).collect();
            let mut cache = Counts::default();
            for text in &paragraphs {
                cache.remember_with_limits(text, 2, max_entries, max_text);
            }
            let retained = cache.words.len();
            let bytes = cache.bytes;
            assert!(retained > 0 && retained < paragraphs.len());
            let mut hits = 0;
            for _ in 0..3 {
                for text in &paragraphs {
                    hits += usize::from(cache.words.contains_key(text));
                    cache.remember_with_limits(text, 2, max_entries, max_text);
                }
                assert_eq!((cache.words.len(), cache.bytes), (retained, bytes));
            }
            assert_eq!(hits, retained * 3, "a sequential scan cannot evict all earlier hits");
            assert!(cache.words.len() <= max_entries && cache.bytes <= max_text);
            assert_eq!(cache.bytes, cache.words.keys().map(String::len).sum::<usize>());
        }
    }
    #[test]
    fn repeated_large_document_counts_segment_only_changed_paragraphs() {
        if !wordcraft_proof::khmer::data_available() {
            return;
        }
        // This goes through Document::word_count, not just cache insertion, and
        // exceeds the old 4,096-entry limit several times over.
        let paragraphs: Vec<_> = (0..20_000).map(|n| format!("ខ្មែរ full-scan-regression-{n}")).collect();
        let mut doc = crate::Document::from_text(&paragraphs.join("\n"));
        let calls = || SEGMENTATIONS.with(std::cell::Cell::get);
        let before = calls();
        assert_eq!(doc.word_count(), 40_000);
        assert_eq!(calls() - before, paragraphs.len(), "cold count segments each paragraph once");
        let warm = calls();
        assert_eq!(doc.word_count(), 40_000);
        assert_eq!(doc.word_count(), 40_000);
        assert_eq!(calls(), warm, "warm full scans reuse every unchanged paragraph");
        doc.para_mut(crate::StoryRef::Body, &crate::Path::top(0)).unwrap().text.push_str(" extra");
        assert_eq!(doc.word_count(), 40_001);
        assert_eq!(calls(), warm + 1, "an edit segments only the changed paragraph");
        assert_eq!(doc.word_count(), 40_001);
        assert_eq!(calls(), warm + 1, "the edited paragraph is cached for subsequent scans too");
        eprintln!("large-document cache: {} cold segmentations; 0 on repeated scans; 1 after an edit", warm - before);
    }
    #[test]
    fn mixed_scripts_numbers_and_newlines_keep_their_counts() {
        let khmer = if wordcraft_proof::khmer::data_available() { 4 } else { 1 };
        // Latin touching Khmer, Khmer digits, ASCII digits, CJK and punctuation.
        assert_eq!(count_words(&format!("Hello{PHRASE}world ១២៣ 42 我们 日本។")), khmer + if khmer == 4 { 8 } else { 6 });
        assert_eq!(count_words(&format!("{PHRASE}\r\n{PHRASE}\n")), khmer * 2);
        for (text, n) in
            [("Hello world", 2), ("Bonjour élève", 2), ("مرحبا بالعالم", 2), ("ภาษาไทย", 1), ("안녕하세요 세계", 2), ("WordCraft是一个", 4)]
        {
            assert_eq!(count_words(text), n, "{text}");
        }
    }
    #[test]
    fn counts_track_changed_text_and_do_not_mutate_documents() {
        let mut doc = crate::Document::from_text(PHRASE);
        let before = doc.clone();
        let expected = if wordcraft_proof::khmer::data_available() { 4 } else { 1 };
        assert_eq!(doc.word_count(), expected);
        assert_eq!(doc, before);
        let path = crate::Path::top(0);
        doc.para_mut(crate::StoryRef::Body, &path).unwrap().text = format!("{PHRASE} {PHRASE}");
        assert_eq!(doc.word_count(), expected * 2, "public text changes cannot use stale cached counts");
        assert_eq!(before.word_count(), expected, "an undo snapshot keeps its original count");
    }
}
