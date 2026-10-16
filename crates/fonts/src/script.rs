//! Scripts: itemising text into single-script runs for the shaper, and the fonts preferred when
//! a document's font lacks a complex script (Myanmar, Khmer, the Indic scripts, Thai…).
//!
//! A shaper picks its script-specific engine (the Myanmar, Khmer, Indic or Universal Shaping
//! Engine reordering, e.g. U+1031 ေ drawn before its consonant) from the run's script, so text
//! mixing scripts in one face must be shaped one script at a time.

#![allow(clippy::single_range_in_vec_init)] // one run is a list of runs

use unicode_script::{Script, UnicodeScript};

/// The script of `c` for itemisation: `None` for characters that take the script of the text
/// around them (spaces, punctuation, digits, combining marks, joiners).
fn real_script(c: char) -> Option<Script> {
    match c.script() {
        Script::Common | Script::Inherited | Script::Unknown => None,
        // Japanese and Chinese mix these within a word: one run.
        Script::Hiragana | Script::Katakana | Script::Bopomofo => Some(Script::Han),
        s => Some(s),
    }
}

/// Maximal single-script byte ranges of `text`. Characters of no particular script join the run
/// before them (or, at the start, the first run), so the ranges cover `text` exactly.
pub(crate) fn script_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.is_ascii() {
        return vec![0..text.len()];
    }
    let mut runs = Vec::new();
    let mut cur: Option<Script> = None;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let Some(s) = real_script(c) else { continue };
        match cur {
            Some(p) if p != s => {
                runs.push(start..i);
                start = i;
                cur = Some(s);
            }
            None => cur = Some(s),
            _ => {}
        }
    }
    runs.push(start..text.len());
    runs
}

/// Scripts whose shaping reorders, stacks or forms clusters (OpenType script-specific engines),
/// so a fallback font must carry OpenType layout tables to draw them properly.
pub fn needs_layout_tables(c: char) -> bool {
    matches!(
        c.script(),
        Script::Myanmar
            | Script::Khmer
            | Script::Devanagari
            | Script::Bengali
            | Script::Gurmukhi
            | Script::Gujarati
            | Script::Oriya
            | Script::Tamil
            | Script::Telugu
            | Script::Kannada
            | Script::Malayalam
            | Script::Sinhala
            | Script::Thai
            | Script::Lao
            | Script::Tibetan
            | Script::Tai_Tham
            | Script::Balinese
            | Script::Javanese
            | Script::Arabic
            | Script::Syriac
    )
}

/// Families preferred, in order, for a character of `c`'s script when the document's font lacks
/// it: the platforms' own OpenType fonts and the open Noto/SIL families. Apple's older fonts for
/// these scripts carry only AAT tables (which the shaper doesn't read), so they come last.
pub fn script_fallbacks(c: char) -> &'static [&'static str] {
    match c.script() {
        Script::Myanmar => &["Myanmar Text", "Noto Sans Myanmar", "Padauk", "Pyidaungsu", "Noto Serif Myanmar", "Myanmar Sangam MN", "Myanmar MN"],
        Script::Khmer => &["Leelawadee UI", "Khmer UI", "Noto Sans Khmer", "Noto Serif Khmer", "Khmer OS", "Khmer Sangam MN", "Khmer MN"],
        Script::Devanagari => &["Nirmala UI", "Mangal", "Kohinoor Devanagari", "Noto Sans Devanagari", "Noto Serif Devanagari", "Lohit Devanagari"],
        Script::Bengali => &["Nirmala UI", "Vrinda", "Kohinoor Bangla", "Noto Sans Bengali", "Noto Serif Bengali", "Lohit Bengali"],
        Script::Thai => &["Leelawadee UI", "Leelawadee", "Tahoma", "Thonburi", "Noto Sans Thai", "Noto Serif Thai"],
        // Hangul: a Korean face rather than whichever Chinese or Japanese CJK font also has it.
        Script::Hangul => &["Malgun Gothic", "Apple SD Gothic Neo", "Noto Sans CJK KR", "Noto Sans KR", "Source Han Sans K", "NanumGothic"],
        Script::Lao => &["Leelawadee UI", "Lao UI", "Noto Sans Lao", "Noto Serif Lao", "Lao Sangam MN"],
        Script::Tamil | Script::Telugu | Script::Kannada | Script::Malayalam | Script::Gujarati | Script::Gurmukhi | Script::Oriya => &["Nirmala UI"],
        _ => &[],
    }
}

/// How well `family` suits `c` as a fallback: 0 for the first preferred family, then the others
/// in order; then families named after the script ("Noto Sans Tamil"); then everything else.
pub fn fallback_rank(c: char, family: &str) -> usize {
    let list = script_fallbacks(c);
    if let Some(i) = list.iter().position(|n| family.eq_ignore_ascii_case(n)) {
        return i;
    }
    if needs_layout_tables(c) {
        let name = c.script().full_name().replace('_', " ");
        if family.to_ascii_lowercase().contains(&name.to_ascii_lowercase()) {
            return list.len();
        }
    }
    list.len() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_split_by_script_and_neutrals_join_their_neighbours() {
        assert_eq!(script_runs("Hello"), vec![0..5]);
        // "Hi မြန်မာ!" → Latin "Hi ", Myanmar "မြန်မာ!".
        let s = "Hi မြန်မာ!";
        let runs = script_runs(s);
        assert_eq!(runs, vec![0..3, 3..s.len()]);
        // A leading neutral joins the first run; marks never start a run.
        assert_eq!(script_runs("(ကေ)"), vec![0.."(ကေ)".len()]);
        // Japanese kana and kanji stay together.
        assert_eq!(script_runs("日本語のテキスト").len(), 1);
        assert_eq!(script_runs(""), vec![0..0]);
    }

    #[test]
    fn complex_scripts_prefer_their_own_fonts() {
        assert!(needs_layout_tables('မ') && needs_layout_tables('ក') && needs_layout_tables('क'));
        assert!(!needs_layout_tables('a') && !needs_layout_tables('語'));
        assert_eq!(fallback_rank('မ', "Myanmar Text"), 0);
        assert!(fallback_rank('မ', "Noto Sans Myanmar") < fallback_rank('မ', "Myanmar Sangam MN"));
        assert!(fallback_rank('த', "Noto Sans Tamil") < fallback_rank('த', "Arial"));
        assert_eq!(fallback_rank('a', "Arial"), fallback_rank('a', "Noto Sans"));
        // Hangul looks for a Korean face first; Han ideographs keep the general order.
        assert_eq!(script_fallbacks('한').first(), Some(&"Malgun Gothic"));
        assert!(script_fallbacks('語').is_empty());
    }
}
