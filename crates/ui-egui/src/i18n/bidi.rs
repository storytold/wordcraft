//! Right-to-left interface text in display order for egui.
//!
//! egui shapes text in runs of one font and places the runs left to right. An Arabic word is one
//! run, which the shaper draws right to left, but the spaces between words come from runs of their
//! own, so a phrase came out with every word correct and the word order reversed (`الحفظ التلقائي`
//! read as `التلقائي الحفظ`). [`display`] puts a line into display order with the Unicode
//! Bidirectional Algorithm before egui lays it out: right-to-left words stay in logical order (the
//! shaper reverses their letters itself), everything else in a right-to-left run is reversed around
//! them, brackets are mirrored, and left-to-right runs (Latin names, numbers) keep their order.
//! Ported from PhotoCraft's `bidi_text` (same team, same licence), with the paragraph direction
//! fixed to right-to-left: an Arabic interface string reads right to left as a whole even when it
//! starts with a Latin name (`Zotero: تم`) or a number.
//!
//! For display only: never store the result, search with it, or put it in an editable field.

use std::borrow::Cow;

use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};

/// Does `text` hold a right-to-left letter (Arabic, Hebrew, Syriac, Thaana, N'Ko …)?
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL))
}

/// `text` (an interface string of a right-to-left language) in the order egui should lay it out
/// from left to right, line by line. Text without right-to-left letters comes back unchanged,
/// without allocating.
pub fn display(text: &str) -> Cow<'_, str> {
    if !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        display_line(line, &mut out);
    }
    Cow::Owned(out)
}

fn display_line(line: &str, out: &mut String) {
    if line.is_empty() {
        return;
    }
    if !has_rtl(line) {
        out.push_str(line);
        return;
    }
    let info = BidiInfo::new(line, Some(Level::rtl()));
    for para in &info.paragraphs {
        if para.range.start >= para.range.end {
            continue;
        }
        let (levels, runs) = info.visual_runs(para, para.range.clone());
        for run in runs {
            // Runs lie on character boundaries; a missing slice is skipped, never a panic.
            let Some(slice) = line.get(run.clone()) else { continue };
            if levels.get(run.start).is_some_and(|level| level.is_rtl()) {
                push_reversed(slice, out);
            } else {
                out.push_str(slice);
            }
        }
    }
}

/// A strong right-to-left letter, or any character of a right-to-left script block (Arabic
/// punctuation such as "،" belongs with the word it follows: the same font shapes both).
fn is_rtl(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL) || matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}')
}

/// Characters that stay inside a right-to-left word: its letters, combining marks, and joiners.
fn keeps_with_word(c: char) -> bool {
    is_rtl(c) || matches!(bidi_class(c), BidiClass::NSM | BidiClass::BN)
}

/// Append a right-to-left run in left-to-right display order: right-to-left words stay whole and
/// in logical order, every other character is reversed around them, and brackets are mirrored.
fn push_reversed(run: &str, out: &mut String) {
    let mut units: Vec<&str> = Vec::new();
    let mut word: Option<usize> = None;
    for (i, c) in run.char_indices() {
        if keeps_with_word(c) {
            word.get_or_insert(i);
            continue;
        }
        if let Some(start) = word.take()
            && let Some(w) = run.get(start..i)
        {
            units.push(w);
        }
        if let Some(single) = run.get(i..i + c.len_utf8()) {
            units.push(single);
        }
    }
    if let Some(start) = word
        && let Some(w) = run.get(start..)
    {
        units.push(w);
    }
    for unit in units.into_iter().rev() {
        let mut chars = unit.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if !keeps_with_word(c) => out.push(mirror(c)),
            _ => out.push_str(unit),
        }
    }
}

/// The mirrored form of a bracket shown in right-to-left text.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        other => other,
    }
}

/// A placeholder stand-in for [`display_template`]: a private-use character, which the bidi
/// algorithm treats as one left-to-right letter, so an inserted value moves as a unit.
fn sentinel(i: usize) -> Option<char> {
    char::from_u32(0xF0000 + u32::try_from(i).ok()?)
}

/// `pieces` (literal text and filled values, in logical order) in display order: the literal text
/// is reordered and each value is kept whole where its placeholder lands. Values are either
/// left-to-right (numbers, paths, names) or already in display order (translated labels).
pub fn display_template(pieces: &[(bool, &str)]) -> String {
    let mut marked = String::new();
    let mut values = Vec::new();
    for (is_value, text) in pieces {
        if *is_value && let Some(s) = sentinel(values.len()) {
            marked.push(s);
            values.push(*text);
        } else {
            marked.push_str(text);
        }
    }
    let shown = display(&marked);
    let mut out = String::with_capacity(shown.len());
    for c in shown.chars() {
        match (c as u32).checked_sub(0xF0000).and_then(|i| values.get(i as usize)) {
            Some(v) => out.push_str(v),
            None => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_to_right_text_is_untouched_and_not_copied() {
        for text in ["WordCraft", "Heading 1", "", "APA", "日本語"] {
            assert!(matches!(display(text), Cow::Borrowed(t) if t == text), "{text}");
        }
    }

    #[test]
    fn arabic_words_are_placed_right_to_left() {
        assert_eq!(display("الحفظ التلقائي"), "التلقائي الحفظ");
        assert_eq!(display("إدراج جدول جديد"), "جديد جدول إدراج");
    }

    #[test]
    fn punctuation_and_ellipsis_move_to_the_left_end() {
        assert_eq!(display("خيارات…"), "…خيارات");
        assert_eq!(display("احفظ المستند."), ".المستند احفظ");
        assert_eq!(display("العنوان:"), ":العنوان");
    }

    #[test]
    fn latin_names_and_numbers_keep_their_order_inside_arabic() {
        assert_eq!(display("حول WordCraft"), "WordCraft حول");
        assert_eq!(display("تباعد أسطر 1.5"), "1.5 أسطر تباعد");
        assert_eq!(display("تصدير PDF"), "PDF تصدير");
    }

    #[test]
    fn brackets_are_mirrored_and_lines_kept() {
        assert_eq!(display("الأحرف (مع المسافات)"), "(المسافات مع) الأحرف");
        assert_eq!(display("إدراج\nجدول"), "إدراج\nجدول");
        assert_eq!(display("تتبع\nالتغييرات ✓"), "تتبع\n✓ التغييرات");
    }

    #[test]
    fn values_stay_whole_where_their_placeholder_lands() {
        // "{n} سطر": the number reads first, at the right.
        assert_eq!(display_template(&[(true, "12"), (false, " سطر")]), "سطر 12");
        // A value that is itself a display-order Arabic label is not reordered again.
        let label = display("إدراج جدول");
        assert_eq!(display_template(&[(false, "يستخدمه الآن "), (true, &label)]), format!("{label} الآن يستخدمه"));
        assert_eq!(display_template(&[(false, "الصفحة "), (true, "2"), (false, " من "), (true, "10")]), "10 من 2 الصفحة");
    }
}
