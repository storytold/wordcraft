mod ngram;

use core::fmt;
use core::hash::BuildHasher;

use crate::{
    alloc::{borrow::Cow, string::String, vec::Vec},
    checker::{Checker, Forceucase, HiddenHomonym},
    classify_casing, Casing, AT_COMPOUND_BEGIN, MAX_WORD_LEN,
};

macro_rules! has_flag {
    ( $flags:expr, $flag:expr ) => {{
        match $flag {
            Some(flag) => $flags.contains(&flag),
            None => false,
        }
    }};
}

/// A wrapper struct for a dictionary that allows customizing suggestion behavior.
///
/// Currently only [ngram suggestions](Suggester::with_ngram_suggestions) may be configured.
#[derive(Clone, Copy)]
pub struct Suggester<'a, S: BuildHasher> {
    checker: Checker<'a, S>,
    ngram_suggest: bool,
}

impl<S: BuildHasher> fmt::Debug for Suggester<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Suggester")
            .field("ngram_suggest", &self.ngram_suggest)
            .finish_non_exhaustive()
    }
}

impl<'a, S: BuildHasher> Checker<'a, S> {
    /// Converts the [Checker] into a [Suggester] with default suggester options set.
    pub fn into_suggester(self) -> Suggester<'a, S> {
        Suggester::new(self)
    }
}

impl<'a, S: BuildHasher> Suggester<'a, S> {
    fn new(checker: Checker<'a, S>) -> Self {
        Self {
            checker,
            ngram_suggest: true,
        }
    }

    /// Returns the underlying [Checker] used by this suggester.
    pub fn into_checker(self) -> Checker<'a, S> {
        self.checker
    }

    /// Enables or disables the suggester from finding suggestions based on "ngram similarity."
    ///
    /// Ngram similarity is a bespoke string similarity metric used by the suggester to find
    /// suggestions when string edits don't produce any likely candidates. Finding a suggestion
    /// with ngram similarity involves iterating through the dictionary's wordlist and therefore
    /// ngram suggestion can be very slow depending on the dictionary size.
    ///
    /// Ngram suggestion is enabled by default.
    pub fn with_ngram_suggestions(mut self, ngram_suggest: bool) -> Self {
        self.ngram_suggest = ngram_suggest;
        self
    }

    /// Fills the given vec with possible corrections from the dictionary for the given word.
    ///
    /// The `out` vec is cleared before suggestions are added.
    pub fn suggest(&self, word: &str, out: &mut Vec<String>) {
        out.clear();
        if word.len() > MAX_WORD_LEN {
            return;
        }

        self.suggest_impl(word, out);
    }

    fn suggest_impl(&self, word: &str, out: &mut Vec<String>) {
        if word.is_empty() {
            return;
        }

        // ICONV
        let word = self.checker.aff.input_conversions.convert(word);
        let casing = classify_casing(&word);
        let mut hq_suggestions = false;

        match casing {
            Casing::None => {
                // ?
                if self
                    .checker
                    .aff
                    .options
                    .compound_force_uppercase_flag
                    .is_some()
                    && self
                        .checker
                        .check_compound::<AT_COMPOUND_BEGIN>(&word, Forceucase::AllowBadForceucase)
                        .is_some()
                {
                    out.push(self.checker.aff.options.case_handling.titlecase(&word));
                    return;
                }
                hq_suggestions |= self.suggest_low(&word, out);
            }
            Casing::Init => {
                hq_suggestions |= self.suggest_low(&word, out);
                let word = self.checker.aff.options.case_handling.lowercase(&word);
                hq_suggestions |= self.suggest_low(&word, out);
            }
            Casing::Camel | Casing::Pascal => {
                hq_suggestions |= self.suggest_low(&word, out);

                if let Some(dot_idx) = word.find('.') {
                    let after_dot = &word[dot_idx + 1..];
                    let casing_after_dot = classify_casing(after_dot);
                    if matches!(casing_after_dot, Casing::Init) {
                        let mut buffer = String::from(word.as_ref());
                        buffer.insert(dot_idx + 1, ' ');
                        // Nuspell inserts suggestions at the beginning of the list in this block.
                        // `insert_sug_first(word, out)`
                        out.insert(0, buffer);
                    }
                }

                // Note: for these next few blocks we `suggest_low` and then insert the suggestion
                // at the beginning of `out` if it checks. This has the opposite ordering as
                // Nuspell and we swap the ordering for borrowing reasons.
                if matches!(casing, Casing::Pascal) {
                    let lowered = self
                        .checker
                        .aff
                        .options
                        .case_handling
                        .lower_first_char(&word);

                    hq_suggestions |= self.suggest_low(&lowered, out);
                    if self.checker.check(&lowered) {
                        out.insert(0, lowered);
                    }
                }

                let lowercase = self.checker.aff.options.case_handling.lowercase(&word);
                hq_suggestions |= self.suggest_low(&lowercase, out);
                if self.checker.check(&lowercase) {
                    out.insert(0, lowercase);
                }

                if matches!(casing, Casing::Pascal) {
                    let titlecase = self.checker.aff.options.case_handling.titlecase(&word);
                    hq_suggestions |= self.suggest_low(&titlecase, out);
                    if self.checker.check(&titlecase) {
                        out.insert(0, titlecase);
                    }
                }

                for i in 0..out.len() {
                    let suggestion = &mut out[i];
                    let Some(after_space_idx) = suggestion.find(' ').map(|idx| idx + 1) else {
                        continue;
                    };
                    let len = suggestion.len() - after_space_idx;
                    if word
                        .len()
                        .checked_sub(len)
                        .and_then(|s| word.get(s..))
                        .is_some_and(|tail| suggestion[after_space_idx..] == *tail)
                    {
                        continue;
                    }
                    let titled = self
                        .checker
                        .aff
                        .options
                        .case_handling
                        .upper_char_at(suggestion, after_space_idx);
                    out[i] = titled;
                    // Rotate this suggestion to the front.
                    if i > 0 {
                        out[..=i].rotate_right(1);
                    }
                }
            }
            Casing::All => {
                let lower = self.checker.aff.options.case_handling.lowercase(&word);
                if self.checker.aff.options.keep_case_flag.is_some() && self.checker.check(&lower) {
                    out.insert(0, lower.clone());
                }
                hq_suggestions |= self.suggest_low(&lower, out);
                let title = self.checker.aff.options.case_handling.titlecase(&lower);
                hq_suggestions |= self.suggest_low(&title, out);
                for suggestion in out.iter_mut() {
                    let upper = self.checker.aff.options.case_handling.uppercase(suggestion);
                    *suggestion = upper;
                }
            }
        }

        if self.ngram_suggest
            && !hq_suggestions
            && self.checker.aff.options.max_ngram_suggestions != 0
        {
            let buffer = if matches!(casing, Casing::None) {
                Cow::Borrowed(word.as_ref())
            } else {
                Cow::Owned(self.checker.aff.options.case_handling.lowercase(&word))
            };
            let old_len = out.len();

            self.ngram_suggest(&buffer, out);

            if matches!(casing, Casing::All) {
                for suggestion in &mut out[old_len..] {
                    let upper = self.checker.aff.options.case_handling.uppercase(suggestion);
                    *suggestion = upper;
                }
            }
        }

        // For words with dashes, if there isn't already a suggestion with dashes, try replacing
        // each of the subwords by running the subword through the suggester.
        let has_dash = word.contains('-');
        let has_dash_sug = has_dash && out.iter().any(|s| s.contains('-'));
        if has_dash && !has_dash_sug {
            // TODO: move this out into its own function?
            let mut suggestions_tmp = Vec::new();
            let mut i = 0;
            loop {
                let j = word[i..].find('-').map(|idx| idx + i);
                let part = match j {
                    Some(j) => &word[i..j],
                    None => &word[i..],
                };
                if !self.checker.check(part) {
                    self.suggest_impl(part, &mut suggestions_tmp);
                    let mut buffer = String::with_capacity(word.len());
                    for s in suggestions_tmp.drain(..) {
                        buffer.clear();
                        buffer.push_str(&word[..i]);
                        buffer.push_str(&s);
                        if let Some(j) = j {
                            buffer.push_str(&word[j..]);
                        }
                        if !self
                            .checker
                            .check_word(&buffer, Forceucase::default(), HiddenHomonym::default())
                            .is_some_and(|flags| {
                                flags.contains(&self.checker.aff.options.forbidden_word_flag)
                            })
                        {
                            out.push(buffer.clone());
                        }
                    }
                }

                match j {
                    Some(j) => i = j + 1,
                    None => break,
                }
            }
        }

        if matches!(casing, Casing::Init | Casing::Pascal) {
            for suggestion in out.iter_mut() {
                let uppered = self
                    .checker
                    .aff
                    .options
                    .case_handling
                    .upper_char_at(suggestion, 0);
                *suggestion = uppered;
            }
        }

        // Nuspell:
        // > Suggest with dots can go here but nobody uses it so no point in
        // > implementing it.

        // NOTE: Both Hunspell and Nuspell check that the forbidden word flag is not zero (None)
        // like the following code:
        //     if matches!(casing, Casing::Init | Casing::All)
        //         && (self.checker.aff.options.keep_case_flag.is_some()
        //             || self.checker.aff.options.forbidden_word_flag.is_some())
        // However the forbidden word flag has a non-zero default, so this `&&` branch always
        // evaluates to `true`. So the check is not done at all here.
        if matches!(casing, Casing::Init | Casing::All) {
            // Happily this is cleaner in Rust because of `retain_mut`. Nuspell inlines
            // `remove_if(it, last, is_not_ok)` because it needs to modify the suggestions.
            out.retain_mut(|suggestion| {
                if suggestion.contains(' ') {
                    return true;
                }
                if self.checker.check(suggestion) {
                    return true;
                }
                let lower = self.checker.aff.options.case_handling.lowercase(suggestion);
                if self.checker.check(&lower) {
                    *suggestion = lower;
                    return true;
                }
                let title = self.checker.aff.options.case_handling.titlecase(suggestion);
                if self.checker.check(&title) {
                    *suggestion = title;
                    return true;
                }
                false
            });
        }

        // Some suggestion methods can cause duplicates. For example in "adveenture",
        // extra_char_suggest can eliminate either inner 'e' causing a duplicate "adventure"
        // suggestion. Deduplicate the results while preserving order:
        deduplicate(out);

        // OCONV
        for suggestion in out.iter_mut() {
            match self.checker.aff.output_conversions.convert(suggestion) {
                Cow::Borrowed(_) => (),
                Cow::Owned(converted) => *suggestion = converted,
            }
        }
    }

    fn suggest_low(&self, word: &str, out: &mut Vec<String>) -> bool {
        let old_len = out.len();

        self.uppercase_suggest(word, out);
        self.rep_suggest(word, out);
        self.map_suggest(word, out);

        let hq_suggestions = out.len() != old_len
            // Per Nuspell we recognize our suggestions so far as high quality if uppercase,
            // rep or map suggestion methods produces a suggestion, or if the word is correct.
            // In Hunspell the map_suggest equivalent would add `word` so the `out.len() !=
            // old_len` check would be the same here.
            || (!self.checker.aff.similarities.is_empty()
                && self
                    .checker
                    .check_word(
                        word,
                        Forceucase::AllowBadForceucase,
                        HiddenHomonym::SkipHiddenHomonym,
                    )
                    .is_some());

        self.adjacent_swap_suggest(word, out);
        self.distant_swap_suggest(word, out);
        self.keyboard_suggest(word, out);
        self.extra_char_suggest(word, out);
        self.forgotten_char_suggest(word, out);
        self.move_char_suggest(word, out);
        self.wrong_char_suggest(word, out);
        self.doubled_two_chars_suggest(word, out);
        self.two_words_suggest(word, out);

        hq_suggestions
    }

    /// Adds a suggestion to the suggestion vector if it belongs to the dictionary and is not
    /// forbidden.
    ///
    /// About accepting `Into<Cow<str>>`: some callers have `String`s that need to be borrowed for
    /// lookup but can be pushed to the output vector without cloning. Some callers are `&str`s
    /// that need to be allocated to `String`s before being pushed. (We should do that conversion
    /// lazily though as the word might be incorrect.) The `Cow` covers both. (Also note the
    /// explicit lifetime but that's only because anonymous lifetimes are not allowed in this slot
    /// as of the current Rust version.)
    fn add_suggestion_if_correct<'w, W: Into<Cow<'w, str>>>(
        &self,
        word: W,
        out: &mut Vec<String>,
    ) -> bool {
        let word = word.into();
        let Some(flags) = self.checker.check_word(
            word.as_ref(),
            Forceucase::ForbidBadForceucase,
            HiddenHomonym::SkipHiddenHomonym,
        ) else {
            return false;
        };

        if flags.contains(&self.checker.aff.options.forbidden_word_flag) {
            return false;
        }

        if self.checker.aff.options.forbid_warn
            && has_flag!(flags, self.checker.aff.options.warn_flag)
        {
            return false;
        }

        out.push(word.into_owned());
        true
    }

    /// Suggests the word, uppercased, if valid.
    ///
    /// For example you might type "ansi" and mean "ANSI".
    fn uppercase_suggest(&self, word: &str, out: &mut Vec<String>) {
        let upper = self.checker.aff.options.case_handling.uppercase(word);
        self.add_suggestion_if_correct(upper, out);
    }

    /// Suggests edits to the word based on the replacement table (`REP`) defined in the `.aff`
    /// file.
    ///
    /// REP is used by a dictionary to list common misspellings or similar sounding word parts.
    /// For example in en_US "shun" has the same sound as "tion" which is typically the correct
    /// word part.
    fn rep_suggest(&self, word: &str, out: &mut Vec<String>) {
        // See `Checker::is_rep_similar` for notes about the way this function is written and
        // allocations
        for (from, to) in self.checker.aff.replacements.whole_word_replacements() {
            if word == from {
                self.try_rep_suggestion(to, out);
            }
        }

        if self
            .checker
            .aff
            .replacements
            .has_only_whole_word_replacements()
        {
            return;
        }

        let mut scratch = String::from(word);

        for (from, to) in self.checker.aff.replacements.start_word_replacements() {
            if word.starts_with(from) {
                scratch.replace_range(..from.len(), to);
                self.try_rep_suggestion(&scratch, out);
                scratch.replace_range(..to.len(), from);
            }
        }

        debug_assert_eq!(&scratch, word);

        for (from, to) in self.checker.aff.replacements.end_word_replacements() {
            if word.ends_with(from) {
                let idx = word.len() - from.len();
                scratch.replace_range(idx.., to);
                self.try_rep_suggestion(&scratch, out);
                scratch.replace_range(idx.., from);
            }
        }

        debug_assert_eq!(&scratch, word);

        for (from, to) in self.checker.aff.replacements.any_place_replacements() {
            for (idx, _) in word.match_indices(from) {
                scratch.replace_range(idx..idx + from.len(), to);
                self.try_rep_suggestion(&scratch, out);
                scratch.replace_range(idx..idx + to.len(), from);
            }
        }

        debug_assert_eq!(&scratch, word);
    }

    fn try_rep_suggestion(&self, word: &str, out: &mut Vec<String>) {
        if self.add_suggestion_if_correct(word, out) {
            return;
        }

        // ?
        let Some(mut j) = word.find(' ') else {
            return;
        };
        let mut i = 0;
        loop {
            let part = &word[i..j];
            if self
                .checker
                .check_word(
                    part,
                    Forceucase::ForbidBadForceucase,
                    HiddenHomonym::SkipHiddenHomonym,
                )
                .is_none()
            {
                return;
            }
            i = j + 1;
            match word[i..].find(' ') {
                Some(idx) => j = i + idx,
                None => break,
            }
        }

        out.push(String::from(word));
    }

    /// Suggests swapping out characters and substrings according to the `MAP` rule in the `.aff`
    /// file.
    ///
    /// This is used to swap out diacritics, for example equating 'o' with 'ö'.
    fn map_suggest(&self, word: &str, out: &mut Vec<String>) {
        let mut remaining_attempts = self.max_attempts_for_long_alogs(word);
        self.map_suggest_impl(word, out, 0, &mut remaining_attempts);
    }

    fn map_suggest_impl(
        &self,
        word: &str,
        out: &mut Vec<String>,
        i: usize,
        remaining_attempts: &mut usize,
    ) {
        let buffer = &mut String::from(word);
        for (mut idx, ch) in word[i..].char_indices() {
            idx += i;
            for similarity in self.checker.aff.similarities.iter() {
                if similarity.chars.contains(ch) {
                    for similar_ch in similarity.chars.chars() {
                        if similar_ch == ch {
                            continue;
                        }
                        if *remaining_attempts == 0 {
                            return;
                        }
                        *remaining_attempts -= 1;

                        replace_char_at(buffer, idx, ch, similar_ch);
                        self.add_suggestion_if_correct(&*buffer, out);
                        self.map_suggest_impl(
                            buffer,
                            out,
                            idx + similar_ch.len_utf8(),
                            remaining_attempts,
                        );
                        replace_char_at(buffer, idx, similar_ch, ch);
                        debug_assert_eq!(&*buffer, word);
                    }
                    for similar_str in similarity.strings.iter() {
                        if *remaining_attempts == 0 {
                            return;
                        }
                        *remaining_attempts -= 1;

                        buffer.replace_range(idx..idx + ch.len_utf8(), similar_str);
                        self.add_suggestion_if_correct(&*buffer, out);
                        self.map_suggest_impl(
                            buffer,
                            out,
                            idx + similar_str.len(),
                            remaining_attempts,
                        );
                        let mut ch_str = [0u8; 4];
                        let ch_str = ch.encode_utf8(&mut ch_str);
                        buffer.replace_range(idx..idx + similar_str.len(), ch_str);
                        debug_assert_eq!(&*buffer, word);
                    }
                } else {
                    for string in similarity.strings.iter() {
                        let Some(idx) = word[idx..].find(&**string).map(|i| i + idx) else {
                            continue;
                        };
                        for similar_ch in similarity.chars.chars() {
                            if *remaining_attempts == 0 {
                                return;
                            }
                            *remaining_attempts -= 1;

                            let mut ch_str = [0u8; 4];
                            let ch_str = similar_ch.encode_utf8(&mut ch_str);
                            buffer.replace_range(idx..idx + string.len(), ch_str);
                            self.add_suggestion_if_correct(&*buffer, out);
                            self.map_suggest_impl(
                                buffer,
                                out,
                                idx + ch_str.len(),
                                remaining_attempts,
                            );
                            buffer.replace_range(idx..idx + ch_str.len(), string);
                            debug_assert_eq!(&*buffer, word);
                        }
                        for similar_str in similarity.strings.iter() {
                            if core::ptr::eq(string, similar_str) {
                                continue;
                            }
                            if *remaining_attempts == 0 {
                                return;
                            }
                            *remaining_attempts -= 1;

                            buffer.replace_range(idx..idx + string.len(), similar_str);
                            self.add_suggestion_if_correct(&*buffer, out);
                            self.map_suggest_impl(
                                buffer,
                                out,
                                idx + similar_str.len(),
                                remaining_attempts,
                            );
                            buffer.replace_range(idx..idx + similar_str.len(), string);
                            debug_assert_eq!(&*buffer, word);
                        }
                    }
                }
            }
        }
    }

    /// Suggests swapping two adjacent characters.
    ///
    /// Also suggests some extra swaps for words with exactly 4 or 5 characters.
    fn adjacent_swap_suggest(&self, word: &str, out: &mut Vec<String>) {
        // Nuspell early-returns on this condition but I don't think that's necessary.
        debug_assert!(!word.is_empty());

        let buffer = &mut String::from(word);
        let mut chars = word.char_indices().peekable();
        while let Some(((idx1, ch1), (_idx2, ch2))) = chars.next().zip(chars.peek()) {
            swap_adjacent_chars(buffer, idx1, ch1, *ch2);
            self.add_suggestion_if_correct(&*buffer, out);
            swap_adjacent_chars(buffer, idx1, *ch2, ch1);
            debug_assert_eq!(word, &*buffer);
        }

        // For words with exactly 4 or 5 characters try some additional swaps.
        let mut chars = word.char_indices();
        let Some((idx1, ch1)) = chars.next() else {
            return;
        };
        let Some((idx2, ch2)) = chars.next() else {
            return;
        };
        let Some((idx3, ch3)) = chars.next() else {
            return;
        };
        let Some((idx4, ch4)) = chars.next() else {
            return;
        };
        let Some((_idx5, ch5)) = chars.next() else {
            // Word has 4 characters. Try a double swap: the first two and last two characters.
            swap_adjacent_chars(buffer, idx1, ch1, ch2); // swap 1 & 2
            swap_adjacent_chars(buffer, idx3, ch3, ch4); // swap 3 & 4
            self.add_suggestion_if_correct(&*buffer, out);
            swap_adjacent_chars(buffer, idx1, ch2, ch1); // undo 1 & 2
            swap_adjacent_chars(buffer, idx3, ch4, ch3); // undo 3 & 4
            debug_assert_eq!(word, &*buffer);
            return;
        };
        if chars.next().is_none() {
            // Word has 5 characters. Try two different double swaps: the first two and last two
            // and then the second and third and last two.
            swap_adjacent_chars(buffer, idx1, ch1, ch2); // swap 1 & 2
            swap_adjacent_chars(buffer, idx4, ch4, ch5); // swap 4 & 5
            self.add_suggestion_if_correct(&*buffer, out);
            swap_adjacent_chars(buffer, idx1, ch2, ch1); // undo 1 & 2
            swap_adjacent_chars(buffer, idx2, ch2, ch3); // swap 2 & 3
            self.add_suggestion_if_correct(&*buffer, out);
            swap_adjacent_chars(buffer, idx2, ch3, ch2); // undo 2 & 3
            swap_adjacent_chars(buffer, idx4, ch5, ch4); // undo 4 & 5
            debug_assert_eq!(word, &*buffer);
        };
    }

    /// Suggests swapping two non-adjacent characters in the word.
    fn distant_swap_suggest(&self, word: &str, out: &mut Vec<String>) {
        debug_assert!(!word.is_empty());
        let mut remaining_attempts = self.max_attempts_for_long_alogs(word);
        let buffer = &mut String::from(word);

        for (idx1, ch1) in word.char_indices() {
            for (mut idx2, ch2) in word[idx1..].char_indices().skip(1) {
                // Adjust idx2 so it's an absolute index into word.
                idx2 += idx1;
                if remaining_attempts == 0 {
                    return;
                }
                remaining_attempts -= 1;
                idx2 = swap_distant_chars(buffer, idx1, ch1, idx2, ch2);
                self.add_suggestion_if_correct(&*buffer, out);
                swap_distant_chars(buffer, idx1, ch2, idx2, ch1);
                debug_assert_eq!(word, &*buffer);
            }
        }
    }

    /// Suggests replacing characters in the word with the keys around them on the keyboard.
    ///
    /// Also suggests uppercasing individual characters - this suggests that you meant to hit
    /// shift while typing a character.
    fn keyboard_suggest(&self, word: &str, out: &mut Vec<String>) {
        let kb = &self.checker.aff.keyboard_closeness;
        let mut buffer = String::from(word);

        for (idx, word_ch) in word.char_indices() {
            let mut uppercase = word_ch.to_uppercase();
            if !(uppercase.len() == 1 && uppercase.next() == Some(word_ch)) {
                // Are the indices correct here?
                buffer.truncate(idx);
                buffer.extend(word_ch.to_uppercase());
                buffer.extend(word[idx..].chars().skip(1));
                self.add_suggestion_if_correct(&buffer, out);
                buffer.truncate(idx);
                buffer.push_str(&word[idx..]);
                debug_assert_eq!(word, &buffer);
            }

            for (match_idx, _) in kb.match_indices(word_ch) {
                if let Some(prev_kb_ch) =
                    kb[..match_idx].chars().next_back().filter(|ch| *ch != '|')
                {
                    buffer.remove(idx);
                    buffer.insert(idx, prev_kb_ch);
                    self.add_suggestion_if_correct(&buffer, out);
                    buffer.remove(idx);
                    buffer.insert(idx, word_ch);
                    debug_assert_eq!(word, &buffer);
                }

                if let Some(next_kb_ch) = kb[match_idx + word_ch.len_utf8()..]
                    .chars()
                    .next()
                    .filter(|ch| *ch != '|')
                {
                    buffer.remove(idx);
                    buffer.insert(idx, next_kb_ch);
                    self.add_suggestion_if_correct(&buffer, out);
                    buffer.remove(idx);
                    buffer.insert(idx, word_ch);
                    debug_assert_eq!(word, &buffer);
                }
            }
        }
    }

    /// Suggests edits to the word to drop any character.
    ///
    /// Intuitively you might double-tap a character key on your keyboard.
    fn extra_char_suggest(&self, word: &str, out: &mut Vec<String>) {
        // TODO: many suggestion strategies use a String buffer.
        // Allocate it up front and reuse it for each of these checks?
        for (idx, _ch) in word.char_indices() {
            let mut buffer = String::from(word);
            buffer.remove(idx);
            self.add_suggestion_if_correct(buffer, out);
        }
    }

    /// Suggests edits to the word to add a missing character.
    fn forgotten_char_suggest(&self, word: &str, out: &mut Vec<String>) {
        let mut remaining_attempts = self.max_attempts_for_long_alogs(word);
        let mut buffer = String::from(word);
        for ch in self.checker.aff.try_chars.chars() {
            for idx in word
                .char_indices()
                .map(|(i, _)| i)
                .chain(core::iter::once(word.len()))
            {
                if remaining_attempts == 0 {
                    return;
                }
                remaining_attempts -= 1;
                buffer.insert(idx, ch);
                self.add_suggestion_if_correct(&buffer, out);
                buffer.remove(idx);
            }
        }
    }

    /// Suggests moving any character in the word to any other position in the word.
    fn move_char_suggest(&self, word: &str, out: &mut Vec<String>) {
        let mut attempts = self.max_attempts_for_long_alogs(word);
        for (source, ch) in word.char_indices() {
            for target in word
                .char_indices()
                .map(|(i, _)| i)
                .chain(core::iter::once(word.len()))
            {
                if source == target || source + ch.len_utf8() == target {
                    continue;
                }
                if attempts == 0 {
                    return;
                }
                attempts -= 1;
                let mut candidate = String::from(word);
                candidate.remove(source);
                let target = if target > source {
                    target - ch.len_utf8()
                } else {
                    target
                };
                candidate.insert(target, ch);
                self.add_suggestion_if_correct(&candidate, out);
            }
        }
    }

    /// Suggests words with one "wrong" character swapped for another character from the TRY
    /// alphabet.
    ///
    /// For example this suggests "world" for "warld".
    fn wrong_char_suggest(&self, word: &str, out: &mut Vec<String>) {
        let mut remaining_attempts = self.max_attempts_for_long_alogs(word);
        let mut buffer = String::from(word);
        for ch in self.checker.aff.try_chars.chars() {
            for (idx, word_ch) in word.char_indices() {
                if ch == word_ch {
                    continue;
                }
                if remaining_attempts == 0 {
                    return;
                }
                remaining_attempts -= 1;
                buffer.remove(idx);
                buffer.insert(idx, ch);
                self.add_suggestion_if_correct(&buffer, out);
                buffer.remove(idx);
                buffer.insert(idx, word_ch);
            }
        }
    }

    /// Suggests removing two duplicate characters in a pattern of interspersed characters.
    ///
    /// This suggestion method notices a pattern 'xyxyx' and suggests removing the trailing 'yx'.
    /// ('x' and 'y' are placeholders here - it can be any two characters.)
    fn doubled_two_chars_suggest(&self, word: &str, out: &mut Vec<String>) {
        // To find the 'xyxyx' pattern we set up a sliding window of characters and their byte
        // indices. We shift along the word comparing the elements in that window
        let mut window = [(0, '0'); 5];
        let mut n_chars = 0;
        let mut chars = word.char_indices();
        for (n, (idx, char)) in chars.by_ref().enumerate().take(5) {
            window[n] = (idx, char);
            n_chars = n;
        }
        if n_chars != 4 {
            return;
        }
        for (next_idx, next_char) in chars {
            if window[0].1 == window[2].1
                && window[1].1 == window[3].1
                && window[0].1 == window[4].1
            {
                // This condition is unlikely to be met and is unlikely to occur multiple times
                // in one word. So we don't allocate preemptively outside the loop: allocate a
                // fresh buffer here and don't bother restoring the buffer so it matches `word`.
                // If we decide to pass a `&mut String` buffer for these suggester methods to
                // share this might change.
                let mut buffer = String::from(word);
                buffer.drain(window[3].0..next_idx);
                self.add_suggestion_if_correct(buffer, out);
                // In case we decide to pass a buffer for all suggesters:
                // buffer.insert_str(window[3].0, &word[window[1].0..window[3].0]);
                // debug_assert_eq!(&buffer, word);
            }
            // Slide the window to the next character in the word, discarding the leftmost:
            window.rotate_left(1);
            window[4] = (next_idx, next_char);
        }
    }

    /// Suggests that the input is two correct words from the dictionary mistakenly not separated
    /// by a space or hyphen.
    fn two_words_suggest(&self, word: &str, out: &mut Vec<String>) {
        debug_assert!(!word.is_empty());

        for (n, (idx, _)) in word.char_indices().enumerate() {
            // Note Nuspell has a TODO to consider switching these to `check_word` which would
            // also allow suggesting that either of these words is a compound.
            let word1 = &word[..idx];
            if self
                .checker
                .check_simple_word(word1, HiddenHomonym::SkipHiddenHomonym)
                .is_none()
            {
                continue;
            };
            let word2 = &word[idx..];
            if self
                .checker
                .check_simple_word(word2, HiddenHomonym::SkipHiddenHomonym)
                .is_none()
            {
                continue;
            };

            // This is somewhat unlikely and we would need to clone the string anyways to reuse
            // the buffer so we delay allocating until here (rather than reusing a buffer
            // throughout the loop).
            let mut space_suggestion = String::with_capacity(word1.len() + 1 + word2.len());
            space_suggestion.push_str(word1);
            space_suggestion.push(' ');
            space_suggestion.push_str(word2);
            // Nuspell: if (find(begin(out), end(out), word1) == end(out))
            // is it necessary? we deduplicate at the end of the main suggest function
            out.push(space_suggestion);

            // If '-' is allowed by the TRY characters or if the TRY characters appear Latin and
            // the words are long enough (both more than one char), also include the two words
            // connected by a hyphen.
            let word2_has_more_than_1_char = word2.chars().nth(1).is_some();
            if n > 1
                && word2_has_more_than_1_char
                && self.checker.aff.try_chars.contains(['a', '-'])
            {
                let Some(mut hyphen_suggestion) = out.last().cloned() else {
                    continue;
                };
                hyphen_suggestion.replace_range(idx..idx + 1, "-");
                // Same here about deduplication and the if statement above:
                out.push(hyphen_suggestion);
            }
        }
    }

    /// Determines a reasonable number of attempts to try at editing a word.
    ///
    /// This limits the impact of guessing edits for complicated dictionaries: whether a
    /// dictionary allows compounding and how many prefixes/suffixes a dictionary uses.
    /// (And whether it allows complex prefixes.)
    fn max_attempts_for_long_alogs(&self, word: &str) -> usize {
        let p = self.checker.aff.prefixes.len() as u64 / 20u64;
        let s = self.checker.aff.suffixes.len() as u64 / 20u64;
        let mut cost = 1 + p + s + p * s;
        if self.checker.aff.options.complex_prefixes {
            cost += p * p + 2 * s * p * p;
        } else {
            cost += s * s + 2 * p * s * s;
        }
        cost = cost.clamp(250_000, 25_000_000_000);
        let mut attempts = 25_000_000_000 / cost;
        if self.checker.aff.options.allows_compounding() {
            attempts /= word.len() as u64;
        }
        attempts.try_into().unwrap_or(usize::MAX)
    }
}

/// Removes all duplicate items in a vector while preserving order.
///
/// This works similarly to C++'s iterator `remove(begin, end, value)`: we swap duplicate elements
/// to the end of the vector and then truncate the vector to eliminate the duplicates.
fn deduplicate<T: Eq>(items: &mut Vec<T>) {
    if items.is_empty() {
        return;
    }

    let mut idx = 0;
    let mut last = items.len();
    while idx < last {
        let Some(first) = items[idx + 1..last].iter().position(|i| i == &items[idx]) else {
            idx += 1;
            continue;
        };
        let mut first = first + idx + 1;
        let mut result = first;
        first += 1;
        while first < last {
            if items[first] != items[idx] {
                items.swap(result, first);
                result += 1;
            }
            first += 1;
        }
        idx += 1;
        last = result;
    }

    items.truncate(last);
}

/// Swaps the two given characters in the given string assuming that they are adjacent.
///
/// Adjacency is asserted in debug builds.
fn swap_adjacent_chars(string: &mut String, idx1: usize, ch1: char, ch2: char) -> usize {
    swap_distant_chars(string, idx1, ch1, idx1.saturating_add(ch1.len_utf8()), ch2)
}

/// Swap verified UTF-8 characters, leaving invalid positions unchanged.
fn swap_distant_chars(
    string: &mut String,
    idx1: usize,
    ch1: char,
    idx2: usize,
    ch2: char,
) -> usize {
    let end1 = idx1.saturating_add(ch1.len_utf8());
    let end2 = idx2.saturating_add(ch2.len_utf8());
    if idx1 >= idx2
        || end1 > idx2
        || string.get(idx1..end1).and_then(|s| s.chars().next()) != Some(ch1)
        || string.get(idx2..end2).and_then(|s| s.chars().next()) != Some(ch2)
    {
        return idx2;
    }
    let mut bytes = [0; 4];
    string.replace_range(idx2..end2, ch1.encode_utf8(&mut bytes));
    string.replace_range(idx1..end1, ch2.encode_utf8(&mut bytes));
    idx2 - ch1.len_utf8() + ch2.len_utf8()
}

/// Replace a verified UTF-8 character, leaving invalid positions unchanged.
fn replace_char_at(string: &mut String, idx: usize, ch1: char, ch2: char) {
    let end = idx.saturating_add(ch1.len_utf8());
    if string.get(idx..end).and_then(|s| s.chars().next()) == Some(ch1) {
        let mut bytes = [0; 4];
        string.replace_range(idx..end, ch2.encode_utf8(&mut bytes));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        alloc::{string::ToString, vec},
        Dictionary, TEST_DICT,
    };

    #[test]
    fn deduplicate_test() {
        fn deduplicate<T: Eq, I: Into<Vec<T>>>(items: I) -> Vec<T> {
            let mut items = items.into();
            super::deduplicate(&mut items);
            items
        }

        assert_eq!(deduplicate([1, 1, 2, 2, 3, 3]), vec![1, 2, 3]);
        assert_eq!(deduplicate([1, 2, 3, 3]), vec![1, 2, 3]);
        assert_eq!(deduplicate([1, 2, 3, 2, 3, 1]), vec![1, 2, 3]);
        assert_eq!(deduplicate([1, 1]), vec![1]);
        assert_eq!(deduplicate([1]), vec![1]);
        assert_eq!(deduplicate::<usize, _>([]), vec![]);
    }

    #[test]
    fn swap_adjacent_chars_test() {
        fn swap_adjacent_chars(word: &str, idx: usize) -> String {
            let mut buffer = String::from(word);
            let mut chars = word[idx..].chars();
            let ch1 = chars.next().unwrap();
            let ch2 = chars.next().unwrap();
            super::swap_adjacent_chars(&mut buffer, idx, ch1, ch2);
            buffer
        }

        assert_eq!(swap_adjacent_chars("exmaple", 2), "example".to_string());
        assert_eq!(swap_adjacent_chars("épée", 2), "éépe".to_string());
    }

    #[test]
    fn swap_distant_chars_test() {
        fn swap_distant_chars(word: &str, idx1: usize, idx2: usize) -> String {
            let mut buffer = String::from(word);
            let ch1 = word[idx1..].chars().next().unwrap();
            let ch2 = word[idx2..].chars().next().unwrap();
            super::swap_distant_chars(&mut buffer, idx1, ch1, idx2, ch2);
            buffer
        }

        assert_eq!(swap_distant_chars("example", 1, 4), "epamxle".to_string());
        assert_eq!(swap_distant_chars("iclaér", 0, 4), "éclair".to_string());
        assert_eq!(swap_distant_chars("éclair", 0, 5), "iclaér".to_string());
        assert_eq!(swap_distant_chars("épée", 0, 3), "épée".to_string());
    }

    #[test]
    fn replace_char_at_test() {
        fn replace_char_at<S: ToString>(s: S, idx: usize, ch1: char, ch2: char) -> String {
            let mut s = s.to_string();
            super::replace_char_at(&mut s, idx, ch1, ch2);
            s
        }

        assert_eq!(replace_char_at("bar", 2, 'r', 'z'), "baz".to_string());
        assert_eq!(replace_char_at("hello", 1, 'e', 'é'), "héllo".to_string());
        assert_eq!(replace_char_at("héllo", 1, 'é', 'e'), "hello".to_string());
        assert_eq!(replace_char_at("épée", 0, 'é', 'e'), "epée".to_string());
    }

    fn suggest(dict: &Dictionary, word: &str) -> Vec<String> {
        let mut suggestions = Vec::new();
        let suggester = dict.suggester().with_ngram_suggestions(false);
        suggester.suggest(word, &mut suggestions);
        suggestions
    }

    #[test]
    fn empty_suggest() {
        assert!(suggest(&TEST_DICT, "").is_empty());
    }

    #[test]
    fn huge_word_is_skipped() {
        assert!(suggest(&TEST_DICT, &"hello".repeat(MAX_WORD_LEN)).is_empty());
    }

    #[test]
    fn existing_suggestions_are_cleared() {
        let mut suggestions = Vec::new();
        suggestions.push("example".to_string());
        TEST_DICT.suggest("", &mut suggestions);
        assert!(suggestions.is_empty())
    }

    #[test]
    fn uppercase_suggest() {
        // "ANSI" is correct in en_US and not "ansi".
        assert!(suggest(&TEST_DICT, "ansi").contains(&"ANSI".to_string()));
    }

    #[test]
    fn extra_char_suggest() {
        assert!(suggest(&TEST_DICT, "adveenture").contains(&"adventure".to_string()));
    }

    #[test]
    fn rep_suggest() {
        // Uses the en_US `REP size cise`. Sounds similar but "cise" is correct here.
        assert!(suggest(&TEST_DICT, "exsize").contains(&"excise".to_string()));
    }

    #[test]
    fn forgotten_char_suggest() {
        assert!(suggest(&TEST_DICT, "helo").contains(&"hello".to_string()));
        assert!(suggest(&TEST_DICT, "wrld").contains(&"world".to_string()));
        // trailing position: missing char at end of word
        assert!(suggest(&TEST_DICT, "hell").contains(&"hello".to_string()));
    }

    #[test]
    fn wrong_char_suggest() {
        let suggestions = suggest(&TEST_DICT, "faver");
        assert!(suggestions.contains(&"fiver".to_string()));
        //                              ^
        assert!(suggestions.contains(&"faker".to_string()));
        //                               ^
        assert!(suggestions.contains(&"favor".to_string()));
        //                                ^
    }

    #[test]
    fn keyboard_suggest() {
        let aff = r#"
        KEY qwertyuiop|asdfghjkl|zxcvbnm
        "#;
        let dic = r#"1
        dream
        "#;
        let dict = Dictionary::new(aff, dic).unwrap();

        assert!(suggest(&dict, "dteam").contains(&"dream".to_string()));
        assert!(suggest(&dict, "dresm").contains(&"dream".to_string()));
        assert!(!suggest(&dict, "dredm").contains(&"dream".to_string()));
        // right neighbor: 's' -> 'd' (next char after 's' in the KEY layout)
        assert!(suggest(&dict, "sream").contains(&"dream".to_string()));
    }

    #[test]
    fn adjacent_swap_suggest() {
        // Main part of `adjacent_swap_suggest`: swap any two characters.
        assert!(suggest(&TEST_DICT, "exmaple").contains(&"example".to_string()));
        assert!(suggest(&TEST_DICT, "examlpe").contains(&"example".to_string()));
        // The part at the end of the function: extra swaps for words with 4 or 5 characters.
        // 4 characters double swap - first and last two characters:
        assert!(suggest(&TEST_DICT, "ende").contains(&"need".to_string()));
        // 5 characters double swap - first and last two characters:
        assert!(suggest(&TEST_DICT, "rdema").contains(&"dream".to_string()));
        // 5 characters double swap - second and third and last two characters:
        assert!(suggest(&TEST_DICT, "derma").contains(&"dream".to_string()));
    }

    #[test]
    fn distant_swap_suggest() {
        assert!(suggest(&TEST_DICT, "epamxle").contains(&"example".to_string()));
    }

    #[test]
    fn doubled_two_chars_suggest() {
        assert!(suggest(&TEST_DICT, "bananana").contains(&"banana".to_string()));
        assert!(suggest(&TEST_DICT, "exexecute").contains(&"execute".to_string()));
    }

    #[test]
    fn two_words_suggest() {
        assert!(suggest(&TEST_DICT, "helloworld").contains(&"hello world".to_string()));
        assert!(suggest(&TEST_DICT, "helloworld").contains(&"hello-world".to_string()));
    }

    #[test]
    fn move_char_suggest() {
        // Move the 'o' to the end
        assert!(suggest(&TEST_DICT, "hoell").contains(&"hello".to_string()));
        // move the 'h' to the beginning
        assert!(suggest(&TEST_DICT, "ellho").contains(&"hello".to_string()));

        let aff = r#""#;
        // This word uses each possible UTF-8 length:
        assert_eq!('+'.len_utf8(), 1); // 2b
        assert_eq!('×'.len_utf8(), 2); // c3 97
        assert_eq!('፠'.len_utf8(), 3); // e1 8d a0
        assert_eq!('𝄎'.len_utf8(), 4); // f0 9d 84 8e
        let dic = r#"1
        +×፠𝄎
        "#;
        let dict = Dictionary::new(aff, dic).unwrap();

        assert!(suggest(&dict, "+×፠𝄎").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "×+፠𝄎").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "×፠𝄎+").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "+፠×𝄎").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "+፠𝄎×").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "+×𝄎፠").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "+፠×𝄎").contains(&"+×፠𝄎".to_string()));
        assert!(suggest(&dict, "፠+×𝄎").contains(&"+×፠𝄎".to_string()));
    }

    #[test]
    fn map_suggest() {
        let aff = r#"
        MAP 4
        MAP uúü
        MAP oóö
        MAP ß(ss)
        MAP (foo)(bar)
        "#;
        let dic = r#"6
        hello
        flüme
        strauss
        aßßa
        foobar
        barbar
        "#;
        let dict = Dictionary::new(aff, dic).unwrap();
        assert!(suggest(&dict, "hellö").contains(&"hello".to_string()));
        assert!(suggest(&dict, "helló").contains(&"hello".to_string()));
        assert!(suggest(&dict, "flume").contains(&"flüme".to_string()));
        assert!(suggest(&dict, "strauß").contains(&"strauss".to_string()));
        assert!(suggest(&dict, "assssa").contains(&"aßßa".to_string()));
        assert!(suggest(&dict, "foofoo").contains(&"foobar".to_string()));
        assert!(suggest(&dict, "foofoo").contains(&"barbar".to_string()));
    }

    #[test]
    fn rep_panic() {
        // <https://github.com/blopker/codebook/issues/67>
        let aff = r#"
        FLAG long

        REP 1
        REP è$ e
        "#;
        let dic = r#"6
        café/L'D'Q'
        café/S.
        "#;
        let dict = Dictionary::new(aff, dic).unwrap();
        assert!(suggest(&dict, "caféx").contains(&"café".to_string()));
    }

    #[test]
    fn camel_pascal_multibyte_panic() {
        // "BäB" is Pascal casing. The REP rule produces a space suggestion whose tail byte
        // length does not align to a char boundary in the original word.
        let aff = r#"
        SET UTF-8
        REP 1
        REP ä a_B
        "#;
        let dic = r#"1
        ba
        "#;
        let dict = Dictionary::new(aff, dic).unwrap();
        let mut out = Vec::new();
        dict.suggest("BäB", &mut out);
    }
}
