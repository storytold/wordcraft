//! Justified lines of a document opened from RTF whose document controls carry `\wraptrsp`
//! (`Settings::wrap_trail_spaces`, "Wrap trailing spaces onto the next line"). Word keeps the word
//! that overruns the right edge, squeezing the line's spaces, only when that word's first character
//! overruns the edge by less than half the width of the space before the word (and the spaces keep
//! more than half their width, as without the flag). Without `\wraptrsp` the rule of
//! `rtf_justify_squeeze` stands. In every squeezed RTF line a run of two or more spaces keeps its
//! width; only single spaces shrink.
//!
//! Measured in Microsoft 365 Word (16.0.20527): each probe is a paragraph Word opened from a
//! synthetic RTF file (Letter page, margins 72 pt left / 36 pt right, Courier New, `\qj`, the
//! probe's right indent), and `kept` is whether Word's first line ended with the probe's word.

use super::*;
use wordcraft_doc::props::Align;
use wordcraft_doc::{CharProps, Run};

/// The page's text area in the probes: Letter, 72 pt left and 36 pt right margins.
const LEFT: f32 = 72.0;
const RIGHT: f32 = 576.0;

struct Probe {
    name: &'static str,
    /// Size of the text before and after the word.
    size: f32,
    /// Size of the probe word.
    word_size: f32,
    indent_right: f32,
    /// The first line's text up to the probe word (ends with the space before it).
    before: &'static str,
    word: &'static str,
    /// Whether Word kept `word` on the first line.
    kept: bool,
    /// Whether the RTF file had `\wraptrsp`.
    wraptrsp: bool,
}

const TAIL: &str = " cd cd cd cd cd cd cd cd cd cd cd cd cd cd cd cd end";

#[rustfmt::skip]
const PROBES: &[Probe] = &[
    Probe { name: "w1-P00", size: 12.0, word_size: 12.0, indent_right: 63.80, before: "P00 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "w1-P01", size: 12.0, word_size: 12.0, indent_right: 61.90, before: "P01 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "w1-P02", size: 12.0, word_size: 12.0, indent_right: 61.40, before: "P02 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "w1-P03", size: 12.0, word_size: 12.0, indent_right: 61.00, before: "P03 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "w1-P04", size: 12.0, word_size: 12.0, indent_right: 60.50, before: "P04 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "w1-P05", size: 12.0, word_size: 12.0, indent_right: 59.80, before: "P05 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "w1-P06", size: 12.0, word_size: 12.0, indent_right: 59.40, before: "P06 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "w1-P07", size: 12.0, word_size: 12.0, indent_right: 57.80, before: "P07 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "w1-L00", size: 12.0, word_size: 12.0, indent_right: 61.90, before: "L00 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyzwxyzw", kept: false, wraptrsp: true },
    Probe { name: "w1-L01", size: 12.0, word_size: 12.0, indent_right: 61.40, before: "L01 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyzwxyzw", kept: false, wraptrsp: true },
    Probe { name: "w1-L02", size: 12.0, word_size: 12.0, indent_right: 61.00, before: "L02 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyzwxyzw", kept: true, wraptrsp: true },
    Probe { name: "w1-L03", size: 12.0, word_size: 12.0, indent_right: 60.50, before: "L03 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyzwxyzw", kept: true, wraptrsp: true },
    Probe { name: "w1-L04", size: 12.0, word_size: 12.0, indent_right: 59.40, before: "L04 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyzwxyzw", kept: true, wraptrsp: true },
    Probe { name: "y1-S0", size: 20.0, word_size: 20.0, indent_right: 62.00, before: "S0 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-S1", size: 20.0, word_size: 20.0, indent_right: 63.00, before: "S1 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-S2", size: 20.0, word_size: 20.0, indent_right: 64.00, before: "S2 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-S3", size: 20.0, word_size: 20.0, indent_right: 65.00, before: "S3 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-S4", size: 20.0, word_size: 20.0, indent_right: 65.80, before: "S4 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-S5", size: 20.0, word_size: 20.0, indent_right: 66.40, before: "S5 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "y1-S6", size: 20.0, word_size: 20.0, indent_right: 68.00, before: "S6 ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "y1-M0", size: 12.0, word_size: 24.0, indent_right: 59.60, before: "M0 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-M1", size: 12.0, word_size: 24.0, indent_right: 60.60, before: "M1 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-M2", size: 12.0, word_size: 24.0, indent_right: 61.60, before: "M2 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "y1-M3", size: 12.0, word_size: 24.0, indent_right: 62.60, before: "M3 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "y1-M4", size: 12.0, word_size: 24.0, indent_right: 64.10, before: "M4 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "y1-M5", size: 12.0, word_size: 24.0, indent_right: 65.60, before: "M5 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: false, wraptrsp: true },
    Probe { name: "y1-N0", size: 24.0, word_size: 12.0, indent_right: 110.00, before: "N0 ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-N1", size: 24.0, word_size: 12.0, indent_right: 111.00, before: "N1 ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-N2", size: 24.0, word_size: 12.0, indent_right: 112.00, before: "N2 ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-N3", size: 24.0, word_size: 12.0, indent_right: 113.00, before: "N3 ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "y1-N4", size: 24.0, word_size: 12.0, indent_right: 114.50, before: "N4 ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: true },
    Probe { name: "fit-P00", size: 12.0, word_size: 12.0, indent_right: 68.40, before: "P00 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P01", size: 12.0, word_size: 12.0, indent_right: 65.80, before: "P01 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P02", size: 12.0, word_size: 12.0, indent_right: 64.80, before: "P02 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P03", size: 12.0, word_size: 12.0, indent_right: 64.30, before: "P03 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P04", size: 12.0, word_size: 12.0, indent_right: 63.00, before: "P04 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P05", size: 12.0, word_size: 12.0, indent_right: 61.90, before: "P05 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P06", size: 12.0, word_size: 12.0, indent_right: 61.20, before: "P06 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P07", size: 12.0, word_size: 12.0, indent_right: 60.30, before: "P07 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P08", size: 12.0, word_size: 12.0, indent_right: 59.40, before: "P08 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P09", size: 12.0, word_size: 12.0, indent_right: 58.30, before: "P09 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P10", size: 12.0, word_size: 12.0, indent_right: 57.80, before: "P10 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P11", size: 12.0, word_size: 12.0, indent_right: 57.60, before: "P11 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P12", size: 12.0, word_size: 12.0, indent_right: 57.30, before: "P12 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P13", size: 12.0, word_size: 12.0, indent_right: 56.60, before: "P13 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
    Probe { name: "fit-P14", size: 12.0, word_size: 12.0, indent_right: 54.00, before: "P14 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ", word: "wxyz", kept: true, wraptrsp: false },
];

fn courier(size: f32) -> CharProps {
    CharProps { font: Some("Courier New".into()), size: Some(size), ..Default::default() }
}

/// Lays out `text` (runs of `(len, size)`) as one justified paragraph with right indent
/// `indent_right` on the probes' page; returns its first line (trailing spaces left out), where
/// each of the line's characters starts, and where its last character ends.
fn first_line(text: &str, runs: &[(usize, f32)], indent_right: f32, wraptrsp: bool) -> (String, Vec<f32>, f32) {
    let mut para = wordcraft_doc::Paragraph::with_text(text, courier(runs[0].1));
    para.runs = runs.iter().map(|&(len, size)| Run { len, props: courier(size) }).collect();
    para.props.align = Some(Align::Justify);
    para.props.indent_right = Some(indent_right);
    para.props.space_after = Some(0.0);
    let mut d = Document::new();
    d.last_section.margin_left = LEFT;
    d.last_section.margin_right = 612.0 - RIGHT;
    d.settings.rtf_justification = true;
    d.settings.wrap_trail_spaces = wraptrsp;
    d.body = vec![wordcraft_doc::para_block(para)];
    let l = lay(&d);
    let (para, x) =
        l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, x, .. } = it { Some((para.clone(), *x)) } else { None }).unwrap();
    let line = &para.lines[0];
    let shown = text[line.start..line.stop].trim_end().to_string();
    let starts = (0..shown.len()).map(|i| para.x_of(0, line.start + i).unwrap() + x).collect();
    let end = para.x_of(0, line.start + shown.len()).unwrap() + x;
    (shown, starts, end)
}

fn probe_line(p: &Probe, wraptrsp: bool) -> (String, Vec<f32>, f32) {
    let text = format!("{}{}{}", p.before, p.word, TAIL);
    let runs = [(p.before.len(), p.size), (p.word.len(), p.word_size), (TAIL.len(), p.size)];
    first_line(&text, &runs, p.indent_right, wraptrsp)
}

#[test]
fn the_test_font_has_courier_new_pitch() {
    crate::test_fonts();
    for size in [12.0, 20.0, 24.0] {
        let (_, starts, _) = first_line("abcd efgh", &[(9, size)], 0.0, false);
        let pitch = (starts[5] - starts[0]) / 5.0;
        assert!((pitch - 0.6 * size).abs() < 0.01, "{size} pt: {pitch}");
    }
}

#[test]
fn with_wraptrsp_the_word_stays_only_if_its_first_character_overruns_by_less_than_half_a_space() {
    crate::test_fonts();
    for p in PROBES.iter().filter(|p| p.wraptrsp) {
        let (shown, _, end) = probe_line(p, true);
        let line = format!("{}{}", p.before, p.word);
        let want = if p.kept { line.as_str() } else { p.before.trim_end() };
        assert_eq!(shown, want, "{}: first line", p.name);
        if p.kept {
            let edge = RIGHT - p.indent_right;
            assert!((end - edge).abs() < 0.05, "{}: the line ends at {end}, the edge is {edge}", p.name);
        }
    }
}

#[test]
fn without_wraptrsp_the_rtf_rule_is_unchanged() {
    crate::test_fonts();
    // Word kept every one of these (no `\wraptrsp`), as the rule of `rtf_justify_squeeze` does.
    for p in PROBES.iter().filter(|p| !p.wraptrsp) {
        let (shown, _, end) = probe_line(p, false);
        assert_eq!(shown, format!("{}{}", p.before, p.word), "{}: first line", p.name);
        let edge = RIGHT - p.indent_right;
        assert!((end - edge).abs() < 0.05, "{}: the line ends at {end}, the edge is {edge}", p.name);
    }
    // The probes that wrap with `\wraptrsp` stay on the line without it.
    for p in PROBES.iter().filter(|p| p.wraptrsp && !p.kept && (p.size - p.word_size).abs() < 0.01) {
        let (shown, _, _) = probe_line(p, false);
        let kept_before = (LEFT + p.before.len() as f32 * 0.6 * p.size - 0.6 * p.size) < RIGHT - p.indent_right;
        if kept_before {
            assert_eq!(shown, format!("{}{}", p.before, p.word), "{}: kept without wraptrsp", p.name);
        }
    }
}

/// Word's probe lines with two spaces after each full stop (12 pt, no `\wraptrsp`): right indents
/// and the single-space width Word drew (the double spaces stayed 7.2 pt).
#[rustfmt::skip]
const DOUBLES: &[(&str, f32, f32)] = &[
    ("D0 ab ab ab ab.  ab ab ab ab.  ab ab ab ab.  ab ab ab ab.  ab ab wxyz", 33.10, 5.4733),
    ("D1 ab ab ab ab.  ab ab ab ab.  ab ab ab ab.  ab ab ab ab.  ab ab wxyz", 27.80, 5.8267),
    ("D2 ab ab ab ab.  ab ab ab ab.  ab ab ab ab.  ab ab ab ab.  ab ab wxyz", 25.20, 6.0),
];

#[test]
fn a_squeezed_rtf_line_shrinks_only_its_single_spaces() {
    crate::test_fonts();
    // Word drew these without `\wraptrsp`; with it D0's last word wraps (its first character
    // overruns by 4.3 pt), D1 and D2 stay, and the corpus files with `\wraptrsp` show the same
    // full-width double spaces.
    for wraptrsp in [false, true] {
        for &(line, indent_right, single) in DOUBLES.iter().filter(|d| !wraptrsp || !d.0.starts_with("D0")) {
            let text = format!("{line}{TAIL}");
            let (shown, starts, end) = first_line(&text, &[(text.len(), 12.0)], indent_right, wraptrsp);
            assert_eq!(shown, line, "{line}: Word kept the last word");
            assert!((end - (RIGHT - indent_right)).abs() < 0.05, "{line}: ends at {end}");
            let bytes = line.as_bytes();
            for (i, w) in starts.windows(2).enumerate() {
                if bytes[i] != b' ' {
                    continue;
                }
                let gap = w[1] - w[0];
                let in_run = (i > 0 && bytes[i - 1] == b' ') || bytes.get(i + 1) == Some(&b' ');
                let want = if in_run { 7.2 } else { single };
                assert!((gap - want).abs() < 0.02, "{line} (wraptrsp {wraptrsp}): space at {i} is {gap}, want {want}");
            }
        }
    }
}

#[test]
fn a_line_without_double_spaces_still_shrinks_all_its_spaces_alike() {
    crate::test_fonts();
    for p in PROBES.iter().filter(|p| p.kept && (p.size - p.word_size).abs() < 0.01) {
        let (shown, starts, _) = probe_line(p, p.wraptrsp);
        let line = format!("{}{}", p.before, p.word);
        assert_eq!(shown, line, "{}", p.name);
        let pitch = 0.6 * p.size;
        let spaces = line.matches(' ').count() as f32;
        let ink = (line.len() as f32 - spaces) * pitch;
        let want = (RIGHT - p.indent_right - LEFT - ink) / spaces;
        for (i, w) in starts.windows(2).enumerate() {
            if line.as_bytes()[i] == b' ' {
                assert!((w[1] - w[0] - want).abs() < 0.02, "{}: space at {i} is {}, want {want}", p.name, w[1] - w[0]);
            }
        }
    }
}
