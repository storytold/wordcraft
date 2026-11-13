//! `\jexpand` (expanding justification) turns off the space squeeze Word uses in RTF documents to
//! keep a line's overrunning last word (`Settings::rtf_justification`); `\jcompress` (the default)
//! leaves it on. Measured in Word: with `\jexpand` the word wraps, on left-aligned and justified
//! lines alike. The writer writes `\jexpand` when the document does not squeeze.

use super::*;
use wordcraft_doc::Document;

const NEITHER: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\margl2361\margr446\pard\ql\f0\fs24 Some text.\par}";
const EXPAND: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\margl2361\margr446\jexpand\pard\ql\f0\fs24 Some text.\par}";
const EXPAND_WITH_OTHERS: &str =
    r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\nospaceforul\nolnhtadjtbl\lytprtmet\jexpand\viewkind1\pard\qj\f0\fs24 Some text.\par}";
const COMPRESS: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\jcompress\pard\qj\f0\fs24 Some text.\par}";
const EXPAND_THEN_COMPRESS: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\jexpand\jcompress\pard\ql\f0\fs24 Some text.\par}";
const COMPRESS_THEN_EXPAND: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}\jcompress\jexpand\pard\ql\f0\fs24 Some text.\par}";
const EXPAND_IN_SKIPPED_GROUP: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Courier New;}}{\*\unknowndest\jexpand}\pard\ql\f0\fs24 Some text.\par}";

fn squeezes(rtf: &str) -> bool {
    import(rtf.as_bytes()).unwrap().settings.rtf_justification
}

#[test]
fn jexpand_turns_the_squeeze_off() {
    assert!(!squeezes(EXPAND));
    assert!(!squeezes(EXPAND_WITH_OTHERS));
}

#[test]
fn jcompress_or_neither_word_leaves_it_on() {
    assert!(squeezes(NEITHER));
    assert!(squeezes(COMPRESS));
}

#[test]
fn the_last_of_the_two_words_wins() {
    assert!(squeezes(EXPAND_THEN_COMPRESS));
    assert!(!squeezes(COMPRESS_THEN_EXPAND));
}

#[test]
fn jexpand_in_a_skipped_destination_does_not_count() {
    assert!(squeezes(EXPAND_IN_SKIPPED_GROUP));
}

#[test]
fn jexpand_is_written_once_and_reads_back() {
    let d = import(EXPAND.as_bytes()).unwrap();
    let out = export(&d);
    assert_eq!(out.matches("\\jexpand").count(), 1, "{out}");
    assert!(!out.contains("\\jcompress"), "{out}");
    assert!(!import(out.as_bytes()).unwrap().settings.rtf_justification);
}

#[test]
fn a_squeezing_document_is_written_without_either_word() {
    let out = export(&import(NEITHER.as_bytes()).unwrap());
    assert!(!out.contains("\\jexpand") && !out.contains("\\jcompress"), "{out}");
    assert!(import(out.as_bytes()).unwrap().settings.rtf_justification);
}

#[test]
fn a_document_that_is_not_from_rtf_is_saved_with_jexpand() {
    let d = Document::new();
    assert!(!d.settings.rtf_justification);
    let out = export(&d);
    assert_eq!(out.matches("\\jexpand").count(), 1, "{out}");
    assert!(!import(out.as_bytes()).unwrap().settings.rtf_justification);
}
