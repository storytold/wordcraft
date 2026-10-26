//! The text that `WinitView` shows input methods through `NSTextInputClient`.
//!
//! winit doesn't see the app's text, so the view reports the text inserted since the caret last
//! moved for another reason (a command key, a click, a shortcut, focus), with the caret at its
//! end and the marked text after it. That is enough for the replacements input methods make next
//! to the caret: press-and-hold replaces the letter just typed with an accented one, and dictation
//! replaces the words it just inserted. Dictation also needs a caret to insert at at all.
//!
//! Offsets are UTF-16 code units, as in `NSRange`. They keep growing across resets, so a range an
//! input method computed before a reset never matches text inserted after it.

// `tests/ime_patches.rs` includes this file and provides its own `ImeDeleteSurrounding`.
use super::ImeDeleteSurrounding;

/// Text kept for replacements, in characters.
const KEPT_CHARS: usize = 256;

#[derive(Debug, Default)]
pub(super) struct ImeDocument {
    /// The text inserted since the last reset; the caret is at its end.
    typed: String,
    /// The offset of `typed` in the document.
    start: usize,
}

impl ImeDocument {
    /// The caret's offset, which is also where marked text starts.
    pub fn caret(&self) -> usize {
        self.start + utf16_len(&self.typed)
    }

    /// Forget the text: the caret may have moved.
    pub fn reset(&mut self) {
        self.start = self.caret();
        self.typed.clear();
    }

    /// Text reached the app at the caret.
    pub fn insert(&mut self, text: &str) {
        self.typed.push_str(text);
        let excess = self.typed.chars().count().saturating_sub(KEPT_CHARS);
        if excess > 0 {
            let cut = self.typed.char_indices().nth(excess).map_or(self.typed.len(), |(i, _)| i);
            self.start += utf16_len(&self.typed[..cut]);
            self.typed.drain(..cut);
        }
    }

    /// The selection to report: the caret, or the selection within the marked text.
    pub fn selected_range(&self, marked_selection: Option<(usize, usize)>) -> (usize, usize) {
        match marked_selection {
            Some((location, length)) => (self.caret() + location, length),
            None => (self.caret(), 0),
        }
    }

    /// The known text in `range`, with the range it actually covers.
    pub fn substring(
        &self,
        (location, length): (usize, usize),
    ) -> Option<(String, (usize, usize))> {
        let from = location.max(self.start);
        let to = location.saturating_add(length).min(self.caret());
        if from >= to {
            return None;
        }
        let from_byte = self.byte_offset(from);
        let to_byte = self.byte_offset(to);
        let text = &self.typed[from_byte..to_byte];
        let from = self.start + utf16_len(&self.typed[..from_byte]);
        Some((text.to_owned(), (from, utf16_len(text))))
    }

    /// Remove the text an input method replaces, and return the deletion the app has to make.
    ///
    /// Only a range that reaches the caret is honoured. The part of it after the caret is the
    /// marked text, which the new text replaces anyway. A range that ends before the caret, or
    /// starts before the known text, would need the app's own text and is ignored.
    pub fn replace(&mut self, range: Option<(usize, usize)>) -> Option<ImeDeleteSurrounding> {
        let (location, length) = range?;
        let caret = self.caret();
        if location >= caret || location < self.start || location.saturating_add(length) < caret {
            return None;
        }
        let from = self.byte_offset(location);
        let replaced = self.typed.split_off(from);
        Some(ImeDeleteSurrounding { before_bytes: replaced.len(), after_bytes: 0 })
    }

    /// The byte offset in `typed` of a document offset, moved back to a character boundary.
    fn byte_offset(&self, offset: usize) -> usize {
        let mut units = self.start;
        for (index, c) in self.typed.char_indices() {
            units += c.len_utf16();
            if units > offset {
                return index;
            }
        }
        self.typed.len()
    }
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// What to do with text an input method inserts while there's no composition to commit.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PlainInsert {
    /// The key press being handled reports the text.
    KeyText,
    /// Commit the text: no `KeyboardInput` event carries it.
    Commit,
    /// Drop it.
    Ignore,
}

/// `key_characters` is `Some` while `keyDown:` handles a key, with the characters it types.
pub(super) fn plain_insert(text: &str, key_characters: Option<&str>) -> PlainInsert {
    if text.is_empty() || text.chars().next().is_some_and(char::is_control) {
        return PlainInsert::Ignore;
    }
    match key_characters {
        // An input method that types other text than the key, such as Chinese punctuation (`，`
        // for `,`) or an ideographic space for Space, without a composition.
        Some(characters) if characters != text => PlainInsert::Commit,
        Some(_) => PlainInsert::KeyText,
        // The emoji picker or dictation, outside any key press.
        None => PlainInsert::Commit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> ImeDocument {
        let mut document = ImeDocument::default();
        document.insert(text);
        document
    }

    /// The deletion of `text` before the caret.
    fn deleting(text: &str) -> Option<ImeDeleteSurrounding> {
        Some(ImeDeleteSurrounding { before_bytes: text.len(), after_bytes: 0 })
    }

    #[test]
    fn press_and_hold_replaces_the_letter_before_the_caret() {
        let mut document = typed("café");
        assert_eq!(document.selected_range(None), (4, 0));
        assert_eq!(document.substring((3, 1)), Some(("é".to_owned(), (3, 1))));
        assert_eq!(document.replace(Some((3, 1))), deleting("é"));
        document.insert("è");
        assert_eq!(document.substring((0, 10)), Some(("cafè".to_owned(), (0, 4))));
    }

    #[test]
    fn a_range_into_the_marked_text_replaces_up_to_the_caret() {
        let mut document = typed("e");
        assert_eq!(document.selected_range(Some((1, 0))), (2, 0));
        assert_eq!(document.replace(Some((0, 2))), deleting("e"));
        assert_eq!(document.caret(), 0);
    }

    #[test]
    fn ranges_the_document_cannot_answer_are_ignored() {
        let mut document = typed("abc");
        for range in [None, Some((3, 0)), Some((3, 2)), Some((0, 1)), Some((1, 1))] {
            assert_eq!(document.replace(range), None, "{range:?}");
        }
        document.reset();
        document.insert("x");
        // A range from before the reset.
        assert_eq!(document.replace(Some((2, 1))), None);
        assert_eq!(document.substring((0, 3)), None);
        assert_eq!(document.replace(Some((3, 1))), deleting("x"));
    }

    #[test]
    fn offsets_count_utf16_units() {
        let mut document = typed("a😀b");
        assert_eq!(document.caret(), 4);
        // A range starting inside the surrogate pair takes the whole character.
        assert_eq!(document.replace(Some((2, 2))), deleting("😀b"));
        assert_eq!(document.caret(), 1);
    }

    #[test]
    fn old_text_is_dropped_but_offsets_stay() {
        let mut document = typed(&"a".repeat(KEPT_CHARS));
        document.insert("bc");
        assert_eq!(document.caret(), KEPT_CHARS + 2);
        assert_eq!(document.substring((0, 2)), None);
        assert_eq!(document.replace(Some((KEPT_CHARS, 2))), deleting("bc"));
    }

    #[test]
    fn replacements_delete_exact_bytes_after_accents_flags_and_emoji_sequences() {
        // Toolkits delete a whole grapheme cluster per Backspace, so the deletion is an exact
        // byte count: the text before the replaced range must survive.
        let prefix = "e\u{301}é🇯🇵👨\u{200d}👩\u{200d}👧";
        let prefix_units = utf16_len(prefix);
        assert_eq!(prefix_units, 2 + 1 + 4 + 8);

        // Press-and-hold replaces the letter after the ZWJ sequence.
        let mut document = typed(&format!("{prefix}e"));
        let deletion = document.replace(Some((prefix_units, 1)));
        assert_eq!(deletion, Some(ImeDeleteSurrounding { before_bytes: 1, after_bytes: 0 }));
        assert_eq!(document.substring((0, 100)), Some((prefix.to_owned(), (0, prefix_units))));

        // Replacing the emoji sequence itself: 18 bytes, 8 UTF-16 units.
        let mut document = typed(prefix);
        let deletion = document.replace(Some((7, 8)));
        assert_eq!(deletion, Some(ImeDeleteSurrounding { before_bytes: 18, after_bytes: 0 }));
        assert_eq!(document.substring((0, 100)), Some(("e\u{301}é🇯🇵".to_owned(), (0, 7))));

        // One regional indicator of the flag, and the combining accent without its letter.
        let mut document = typed("a🇯🇵");
        assert_eq!(document.replace(Some((3, 2))), deleting("🇵"));
        assert_eq!(document.substring((0, 10)), Some(("a🇯".to_owned(), (0, 3))));
        let mut document = typed("e\u{301}");
        assert_eq!(document.replace(Some((1, 1))), deleting("\u{301}"));
        assert_eq!(document.substring((0, 10)), Some(("e".to_owned(), (0, 1))));
    }

    #[test]
    fn text_that_differs_from_the_key_is_committed() {
        assert_eq!(plain_insert("，", Some(",")), PlainInsert::Commit);
        assert_eq!(plain_insert("\u{3000}", Some(" ")), PlainInsert::Commit);
        assert_eq!(plain_insert("a", Some("a")), PlainInsert::KeyText);
        assert_eq!(plain_insert("😀", None), PlainInsert::Commit);
        assert_eq!(plain_insert("\r", Some("\r")), PlainInsert::Ignore);
        assert_eq!(plain_insert("", None), PlainInsert::Ignore);
    }
}
