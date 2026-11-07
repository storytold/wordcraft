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
    if !craft_khmer::data_available() {
        return text.split('\u{200b}').fold(0usize, |n, part| n.saturating_add(spaced_words(part)));
    }
    // Each paragraph gets the same refinement budget as line breaking. Joining
    // paragraphs in a selection must not spend one shared budget across a document.
    text.split(['\n', '\r']).fold(0usize, |total, para| total.saturating_add(khmer_words(para)))
}

// Counts recur on edits and statistics requests. Text keys also catch direct
// changes to public document fields. Two generations recover after old documents,
// partial selections and intermediate edits fill a budget: promote previous hits,
// then rotate when the current generation fills. Active working sets that fit a
// generation keep sequential-scan hits, without retaining every historical edit.
// These limits count key bytes, not HashMap buckets or allocator/model overhead.
const MAX_TEXT: usize = 8 * 1024 * 1024; // per generation; 16 MiB total key bytes
const MAX_ENTRIES: usize = 1 << 15; // per generation; 65,536 entries total
const MAX_PARAGRAPH: usize = 64 * 1024;
#[derive(Default)]
struct Generation {
    words: HashMap<String, usize>,
    bytes: usize,
}
struct Counts {
    current: Generation,
    previous: Generation,
    max_entries: usize,
    max_text: usize,
}
impl Default for Counts {
    fn default() -> Self {
        Self::new(MAX_ENTRIES, MAX_TEXT)
    }
}
impl Counts {
    fn new(max_entries: usize, max_text: usize) -> Self {
        Self { current: Generation::default(), previous: Generation::default(), max_entries, max_text }
    }
    fn get(&mut self, text: &str) -> Option<usize> {
        if let Some(words) = self.current.words.get(text) {
            return Some(*words);
        }
        let (key, words) = self.previous.words.remove_entry(text)?;
        self.previous.bytes = self.previous.bytes.saturating_sub(key.len());
        self.insert(key, words);
        Some(words)
    }
    fn remember(&mut self, text: &str, words: usize) {
        // An oversized entry must not rotate out useful paragraphs. A concurrent
        // count may also have inserted this text while segmentation was unlocked.
        if self.max_entries == 0 || text.len() > self.max_text || self.get(text).is_some() {
            return;
        }
        self.insert(text.to_owned(), words);
    }
    fn insert(&mut self, text: String, words: usize) {
        if self.current.words.len() >= self.max_entries || self.current.bytes.saturating_add(text.len()) > self.max_text {
            std::mem::swap(&mut self.current, &mut self.previous);
            self.current.words.clear();
            self.current.bytes = 0;
        }
        self.current.bytes = self.current.bytes.saturating_add(text.len());
        self.current.words.insert(text, words);
    }
}
fn counts() -> &'static Mutex<Counts> {
    static COUNTS: OnceLock<Mutex<Counts>> = OnceLock::new();
    COUNTS.get_or_init(Mutex::default)
}
fn khmer_words(text: &str) -> usize {
    cached_khmer_words(text, counts())
}
fn cached_khmer_words(text: &str, cache: &Mutex<Counts>) -> usize {
    if !text.chars().any(|c| ('\u{1780}'..='\u{17ff}').contains(&c)) {
        return spaced_words(text);
    }
    if let Some(words) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(text) {
        return words;
    }
    // Do the model work outside the lock so exporting cannot block an editor's
    // cache hits for the duration of segmentation.
    let words = segmented_words(text);
    if text.len() <= MAX_PARAGRAPH {
        cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remember(text, words);
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
    for range in craft_khmer::word_ranges(text) {
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
        let available = craft_khmer::data_available();
        assert_eq!(count_words(PHRASE), if available { 4 } else { 1 });
        assert_eq!(count_words("ខ្មែរ\u{200b}ភាសា"), 2, "authored breaks work even without a dictionary");
        assert_eq!(count_words("ៗ។! ា្"), 0, "detached marks/signs are not words");
        assert_eq!(count_words("១២៣"), 1, "Khmer digits are");
        if available {
            // A normal word, a compound, and the repetition sign's preceding word.
            assert_eq!(count_words("អ្នកចម្រៀង ក្រុមហ៊ុន អ្វីៗ"), 2 + 1 + 1);
            let words = craft_khmer::line_breaks(PHRASE).len() + 1;
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
        assert_eq!(cache.current.words.len(), paragraphs.len(), "no entry was cleared before its next use");
        assert!(paragraphs.iter().all(|paragraph| cache.current.words.contains_key(paragraph)));
        let bytes: usize = paragraphs.iter().map(String::len).sum();
        assert_eq!(cache.current.bytes, bytes, "a repeated paragraph is not charged twice");
    }
    #[test]
    fn saturated_cache_recovers_for_new_documents_and_edits() {
        // Exercise both rotation triggers with small limits. Each new working
        // set fits one generation, just as the 20,000-paragraph document does.
        for (max_entries, max_text) in [(4, usize::MAX), (usize::MAX, 80)] {
            let mut cache = Counts::new(max_entries, max_text);
            for i in 0..100 {
                cache.remember(&format!("abandoned edit {i:03}"), i);
            }
            let mut paragraphs: Vec<_> = (0..4).map(|i| format!("new paragraph {i:03}")).collect();
            for text in &paragraphs {
                cache.remember(text, 3);
            }
            // Continue to change the active document, without a process restart.
            for edit in 0..20 {
                for text in &paragraphs {
                    assert_eq!(cache.get(text), Some(3), "sequential scans promote previous-generation hits");
                }
                paragraphs[0] = format!("edited text {edit:03}");
                cache.remember(&paragraphs[0], 3);
                for generation in [&cache.current, &cache.previous] {
                    assert!(generation.words.len() <= max_entries && generation.bytes <= max_text);
                    assert_eq!(generation.bytes, generation.words.keys().map(String::len).sum::<usize>());
                }
                assert!(cache.current.words.keys().all(|key| !cache.previous.words.contains_key(key)), "promotion moves the key without duplication");
            }
            assert!(cache.get("abandoned edit 000").is_none(), "stale text ages out");
        }
    }
    #[test]
    fn recovered_cache_segments_only_new_or_edited_paragraphs() {
        if !craft_khmer::data_available() {
            return;
        }
        let cache = Mutex::new(Counts::new(2048, MAX_TEXT));
        for i in 0..5000 {
            cache.lock().unwrap().remember(&format!("abandoned edit {i}"), 1);
        }
        let mut paragraphs: Vec<_> = (0..2000).map(|i| format!("{}recovered-document-{i}", "ខ្ញុំស្រឡាញ់ភាសាខ្មែរ ".repeat(15))).collect();
        let count = |paragraphs: &[String]| -> usize { paragraphs.iter().map(|text| cached_khmer_words(text, &cache)).sum() };
        let calls = || SEGMENTATIONS.with(std::cell::Cell::get);
        let before = calls();
        assert_eq!(count(&paragraphs), 122_000);
        assert_eq!(calls() - before, 2000);
        let warm = calls();
        for _ in 0..3 {
            assert_eq!(count(&paragraphs), 122_000);
        }
        assert_eq!(calls(), warm, "a new document becomes cacheable after the previous workload filled it");
        paragraphs[0].push_str(" extra");
        assert_eq!(count(&paragraphs), 122_001);
        assert_eq!(calls(), warm + 1);
        assert_eq!(count(&paragraphs), 122_001);
        assert_eq!(calls(), warm + 1);
    }
    #[test]
    fn oversized_entries_do_not_rotate_the_cache() {
        let mut cache = Counts::new(2, 16);
        cache.remember("useful", 1);
        for _ in 0..3 {
            cache.remember("too large for this cache", 5);
        }
        assert_eq!(cache.get("useful"), Some(1));
        assert!(cache.previous.words.is_empty());
        let mut disabled = Counts::new(0, 0);
        disabled.remember("", 0);
        assert!(disabled.get("").is_none());
    }
    #[test]
    fn repeated_large_document_counts_segment_only_changed_paragraphs() {
        if !craft_khmer::data_available() {
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
        let khmer = if craft_khmer::data_available() { 4 } else { 1 };
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
        let expected = if craft_khmer::data_available() { 4 } else { 1 };
        assert_eq!(doc.word_count(), expected);
        assert_eq!(doc, before);
        let path = crate::Path::top(0);
        doc.para_mut(crate::StoryRef::Body, &path).unwrap().text = format!("{PHRASE} {PHRASE}");
        assert_eq!(doc.word_count(), expected * 2, "public text changes cannot use stale cached counts");
        assert_eq!(before.word_count(), expected, "an undo snapshot keeps its original count");
    }
}
