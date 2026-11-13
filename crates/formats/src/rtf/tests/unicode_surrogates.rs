//! `\uN` with N a UTF-16 surrogate (RTF 1.9.1, "Unicode RTF"): a high surrogate pairs only with a low
//! surrogate in the very next `\u` (its `\ucN` fallback characters in between are skipped, as for
//! any `\u`). Any other high or low surrogate stands alone and reads as U+FFFD, as Word shows it.

use super::*;
use wordcraft_doc::StoryRef;

const R: char = '\u{FFFD}';

fn body_text(body: &str) -> String {
    let rtf = format!("{{\\rtf1\\ansi\\ansicpg1252{{\\fonttbl\\f0\\fswiss Helvetica;}}\\pard\\f0\\fs22 {body}\\par\n}}");
    import(rtf.as_bytes()).unwrap().plain_text(StoryRef::Body)
}

#[test]
fn a_surrogate_pair_reads_as_one_character() {
    assert_eq!(body_text("a\\uc1\\u-10179?\\u-8704?b").trim_end(), "a\u{1F600}b");
    assert_eq!(body_text("a\\uc0\\u55357\\u56832 b").trim_end(), "a\u{1F600}b");
}

#[test]
fn a_high_surrogate_followed_by_text_is_a_replacement_character() {
    // As tika's testRTFInvalidUnicode.rtf: Word shows "Unpaired hi \u{FFFD} here".
    assert_eq!(body_text("\\uc0 Unpaired hi \\u-10240  here").trim_end(), format!("Unpaired hi {R} here"));
}

#[test]
fn a_held_high_surrogate_does_not_pair_with_a_later_low_one() {
    // Word: "Unpaired hi \u{FFFD} here Unpaired lo \u{FFFD} here"; never U+10000.
    let got = body_text("\\uc0 Unpaired hi \\u-10240  here Unpaired lo \\u-9216  here");
    assert_eq!(got.trim_end(), format!("Unpaired hi {R} here Unpaired lo {R} here"));
    assert!(!got.contains('\u{10000}'), "{got:?}");
}

#[test]
fn a_low_then_high_surrogate_is_two_replacement_characters() {
    // Word: "Mismatched pair \u{FFFD}\u{FFFD} here".
    assert_eq!(body_text("\\uc0 Mismatched pair \\u-9216\\u-10240  here").trim_end(), format!("Mismatched pair {R}{R} here"));
}

#[test]
fn a_high_surrogate_before_another_pair_is_its_own_replacement_character() {
    assert_eq!(body_text("\\uc1 x\\u-10240?\\u-10179?\\u-8704?y").trim_end(), format!("x{R}\u{1F600}y"));
}

#[test]
fn a_high_surrogate_at_the_end_of_a_paragraph_is_kept_as_a_replacement_character() {
    let rtf = "{\\rtf1\\ansi{\\fonttbl\\f0 Arial;}\\pard\\f0\\uc1 end\\u-10240?\\par\\pard next\\u-9216?\\par\n}";
    let d = import(rtf.as_bytes()).unwrap();
    assert_eq!(d.plain_text(StoryRef::Body), format!("end{R}\nnext{R}"));
}

#[test]
fn multibyte_text_and_a_held_surrogate_keep_their_reading_order() {
    let (a, _, _) = encoding_rs::SHIFT_JIS.decode(&[0x82, 0xa0]);
    let read = |body: &str| {
        let rtf = format!("{{\\rtf1\\ansi{{\\fonttbl\\f0\\fcharset128 MS Mincho;}}\\pard\\f0\\uc0 {body}\\par\n}}");
        import(rtf.as_bytes()).unwrap().plain_text(StoryRef::Body)
    };
    // The space ends `\u-10179`: it is not text.
    assert_eq!(read("A\\'82\\'a0\\u-10179 B").trim_end(), format!("A{a}{R}B"));
    assert_eq!(read("A\\u-10179\\'82\\'a0B").trim_end(), format!("A{R}{a}B"));
}
