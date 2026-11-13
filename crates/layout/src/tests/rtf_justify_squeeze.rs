//! A justified line in a document opened from RTF (`Settings::rtf_justification`) keeps the word
//! that overruns its right edge when the space before that word starts inside the line and the
//! line's spaces keep more than half their width; the spaces then shrink alike so the line ends at
//! the edge. Measured in Microsoft 365 Word: each probe below is a paragraph Word opened from a
//! synthetic RTF file (Courier New, `\qj`, the same words, size and text width), and `kept` is
//! whether Word's first line ended with the probe's last word. The same paragraphs saved as DOCX
//! (compatibility modes 11-15) all wrapped that word, as WordCraft's mode-15 rule does.

use super::*;
use wordcraft_doc::props::Align;

/// The page's text area (Letter, 1" margins): 72..540 pt.
const LEFT: f32 = 72.0;
const RIGHT: f32 = 540.0;

struct Probe {
    name: &'static str,
    size: f32,
    /// Right indent that leaves Word's text width for the probe.
    indent_right: f32,
    /// The words of the first line Word was given; the paragraph goes on after them.
    line: &'static str,
    /// Whether Word kept the last word of `line` on the first line.
    kept: bool,
}

#[rustfmt::skip]
const PROBES: &[Probe] = &[
    Probe { name: "T1", size: 12.0, indent_right: 84.25, line: "abcdefghijk defghijklmn ghijklmnopq jklmnopqrs mnopqrst", kept: true }, // squeeze 0.4253, (O - word) / space -6.299; mode 15 keeps: False
    Probe { name: "T2", size: 12.0, indent_right: 84.25, line: "hi kl no qr tu wx za cd fg ij lm op rs uv xy a d g j m pqrs", kept: false }, // squeeze 0.2851, (O - word) / space 1.701; mode 15 keeps: False
    Probe { name: "T3", size: 12.0, indent_right: 84.25, line: "opqrs rstuv uvwxy xyzab abcd defg ghij jklm mnop pqrs stu", kept: true }, // squeeze 0.3701, (O - word) / space 0.701; mode 15 keeps: False
    Probe { name: "T4", size: 12.0, indent_right: 84.25, line: "vwxy yzab bcde efg hij klm nop qrs tuv wxy zabcdefghijklmno", kept: false }, // squeeze 0.5701, (O - word) / space -10.299; mode 15 keeps: False
    Probe { name: "T5", size: 12.0, indent_right: 84.25, line: "cdefg fghij ijklm lmno opqr rstu uvwx xyza abcd defg gh", kept: true }, // squeeze 0.1701, (O - word) / space -0.299; mode 15 keeps: False
    Probe { name: "T6", size: 12.0, indent_right: 84.25, line: "jklm mnop pqrs stuv vwxy yzab bcde efgh hijk klmn nopqrstu", kept: true }, // squeeze 0.4701, (O - word) / space -3.299; mode 15 keeps: False
    Probe { name: "T7", size: 12.0, indent_right: 84.25, line: "qrstu tuvw wxyz zabc cdef fghi ijkl lmno opqr rstu uvwxyzab", kept: false }, // squeeze 0.5701, (O - word) / space -2.299; mode 15 keeps: False
    Probe { name: "T9", size: 12.0, indent_right: 84.25, line: "efghijkl hijklmno klmnopqr nopqrstu qrstuvwx tuvwxyz wx", kept: true }, // squeeze 0.2836, (O - word) / space -0.299; mode 15 keeps: False
    Probe { name: "T10", size: 12.0, indent_right: 84.25, line: "lmn opq rst uvw xyz abc de gh jk mn pq st vw yz bc ef hij", kept: true }, // squeeze 0.2313, (O - word) / space 0.701; mode 15 keeps: False
    Probe { name: "U1", size: 12.0, indent_right: 66.0, line: "ab de gh jk mn pq st vw yz bc ef hi kl no qr t w z c f ijklm", kept: true }, // squeeze 0.2083, (O - word) / space -0.833; mode 15 keeps: False
    Probe { name: "U2", size: 12.0, indent_right: 72.0, line: "hi kl no qr tu wx za cd fg ij lm op rs uv xy a d g j m pqrst", kept: true }, // squeeze 0.25, (O - word) / space 0.0; mode 15 keeps: False
    Probe { name: "U3", size: 12.0, indent_right: 75.0, line: "op rs uv xy ab de gh jk mn pq st vw yz bc ef h k n q t wxyza", kept: true }, // squeeze 0.2708, (O - word) / space 0.417; mode 15 keeps: False
    Probe { name: "U4", size: 12.0, indent_right: 75.0, line: "vw yz bc ef hi kl no q t w z c f i l o r u x a d g j m pqrst", kept: true }, // squeeze 0.2257, (O - word) / space 0.417; mode 15 keeps: False
    Probe { name: "U5", size: 12.0, indent_right: 78.0, line: "cd fg ij lm op rs uv x a d g j m p s v y b e h k n q t wxyza", kept: true }, // squeeze 0.2431, (O - word) / space 0.833; mode 15 keeps: False
    Probe { name: "U6", size: 12.0, indent_right: 75.0, line: "jkl mno pqr stu vwx yza bcd efg hij klm nop qrs tuv wx zabcd", kept: true }, // squeeze 0.3869, (O - word) / space 0.417; mode 15 keeps: False
    Probe { name: "U7", size: 12.0, indent_right: 78.0, line: "qrs tuv wxy zab cde fgh ijk lmn opq rst uvw xyz abc de ghijk", kept: true }, // squeeze 0.4167, (O - word) / space 0.833; mode 15 keeps: False
    Probe { name: "U8", size: 12.0, indent_right: 76.5, line: "xy ab de gh jk mn pq st vw yz bc ef hi kl no q t w z c fghij", kept: true }, // squeeze 0.2812, (O - word) / space 0.625; mode 15 keeps: False
    Probe { name: "U9", size: 20.0, indent_right: 75.0, line: "efg hi kl no qr tu wx za cd fg ijklm", kept: true }, // squeeze 0.325, (O - word) / space -1.75; mode 15 keeps: False
    Probe { name: "U10", size: 20.0, indent_right: 84.0, line: "lmn op rs uv xy ab de gh jk mn pqrst", kept: true }, // squeeze 0.4, (O - word) / space -1.0; mode 15 keeps: False
    Probe { name: "U11", size: 8.0, indent_right: 75.0, line: "stuv vwxy yzab bcde efgh hij klm nop qrs tuv wxy zab cde fgh ijk lmn opq rst uvw xyz abcde", kept: false }, // squeeze 0.4062, (O - word) / space 3.125; mode 15 keeps: False
    Probe { name: "U12", size: 8.0, indent_right: 66.0, line: "za cd fg ij lm op rs uv xy ab de gh jk mn pq st vw yz bc ef hi kl no qr tu w z c f i lmnop", kept: false }, // squeeze 0.2083, (O - word) / space 1.25; mode 15 keeps: False
    Probe { name: "U13", size: 12.0, indent_right: 72.0, line: "gh jk mn p s v y b e h k n q t w z c f i l o r u x a d ghijk", kept: true }, // squeeze 0.1923, (O - word) / space 0.0; mode 15 keeps: False
    Probe { name: "U14", size: 12.0, indent_right: 66.0, line: "nopqrs qrstuv tuvwxy wxyzab zabcde cdefgh fghijk ijklm lmnop", kept: false }, // squeeze 0.5208, (O - word) / space -0.833; mode 15 keeps: False
];

/// The paragraph's first line (trailing spaces left out) and where each of its words starts and
/// its last character ends, in page points.
fn first_line(p: &Probe, rtf: bool) -> (String, Vec<f32>, f32) {
    let chr = wordcraft_doc::CharProps { font: Some("Courier New".into()), size: Some(p.size), ..Default::default() };
    let mut para = wordcraft_doc::Paragraph::with_text(&format!("{} continuation tail text here.", p.line), chr);
    para.props.align = Some(Align::Justify);
    para.props.indent_right = Some(p.indent_right);
    para.props.space_after = Some(0.0);
    let mut d = Document::new();
    d.settings.rtf_justification = rtf;
    d.body = vec![wordcraft_doc::para_block(para)];
    let text = d.body[0].as_para().unwrap().text.clone();
    let l = lay(&d);
    let (para, x) =
        l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, x, .. } = it { Some((para.clone(), *x)) } else { None }).unwrap();
    let line = &para.lines[0];
    let shown = text[line.start..line.stop].trim_end().to_string();
    let mut starts = Vec::new();
    let mut at = line.start;
    for w in shown.split(' ') {
        starts.push(para.x_of(0, at).unwrap() + x);
        at += w.len() + 1;
    }
    let end = para.x_of(0, line.start + shown.len()).unwrap() + x;
    (shown, starts, end)
}

fn without_last_word(line: &str) -> &str {
    &line[..line.rfind(' ').unwrap()]
}

#[test]
fn the_test_font_has_courier_new_pitch() {
    crate::test_fonts();
    for size in [8.0, 12.0, 20.0] {
        let p = Probe { name: "pitch", size, indent_right: 0.0, line: "abcd efgh", kept: true };
        let (_, starts, _) = first_line(&p, false);
        let pitch = (starts[1] - starts[0]) / 5.0;
        assert!((pitch - 0.6 * size).abs() < 0.01, "{size} pt: {pitch}");
    }
}

#[test]
fn an_rtf_justified_line_keeps_or_wraps_its_last_word_as_word_does() {
    crate::test_fonts();
    for p in PROBES {
        let (shown, _, end) = first_line(p, true);
        let want = if p.kept { p.line } else { without_last_word(p.line) };
        assert_eq!(shown, want, "{}: first line", p.name);
        // Kept or wrapped, the line is justified to the right edge (a wrapped line that still
        // overruns, like T2's, is squeezed too).
        let edge = RIGHT - p.indent_right;
        assert!((end - edge).abs() < 0.05, "{}: the line ends at {end}, the edge is {edge}", p.name);
    }
}

#[test]
fn a_kept_word_shrinks_every_space_of_the_line_alike() {
    crate::test_fonts();
    for p in PROBES.iter().filter(|p| p.kept) {
        let (shown, starts, end) = first_line(p, true);
        assert_eq!(shown, p.line, "{}: the last word stays on the line", p.name);
        let pitch = 0.6 * p.size;
        let words: Vec<&str> = shown.split(' ').collect();
        let spaces = (words.len() - 1) as f32;
        let ink: f32 = words.iter().map(|w| w.len() as f32 * pitch).sum();
        let want = (RIGHT - p.indent_right - LEFT - ink) / spaces;
        assert!(want < pitch, "{}: the probe overruns at natural width", p.name);
        assert!(want > 0.5 * pitch, "{}: the spaces keep more than half their width", p.name);
        for (i, pair) in starts.windows(2).enumerate() {
            let gap = pair[1] - pair[0] - words[i].len() as f32 * pitch;
            assert!((gap - want).abs() < 0.02, "{}: space {i} is {gap}, want {want}", p.name);
        }
        assert!((starts[0] - LEFT).abs() < 0.01, "{}: the line starts at the margin", p.name);
        assert!((end - (RIGHT - p.indent_right)).abs() < 0.05, "{}", p.name);
    }
}

#[test]
fn without_rtf_justification_the_mode_15_rule_still_wraps_these_words() {
    crate::test_fonts();
    // None of the probes fits Word 2013's rule (a quarter of the spaces, 0.355 of the word):
    // a DOCX or new document keeps wrapping them, as Word does with the same paragraphs as DOCX.
    for p in PROBES {
        let (shown, _, _) = first_line(p, false);
        assert_ne!(shown, p.line, "{}: mode 15 wraps the last word", p.name);
    }
}

/// Word's rule doesn't cap the squeeze at a number of spaces: with 20 single spaces (Courier New
/// 12 pt, half a space 3.6 pt, Letter page, margins 72 pt left / 36 pt right) the overrunning word
/// stays however long it is, as long as the space before it starts inside the line, left and
/// justified alike. Measured in Microsoft 365 Word from synthetic RTF files: (right indent, the
/// line's last word, kept).
#[rustfmt::skip]
const WORD_LENGTH_PROBES: &[(f32, &str, bool)] = &[
    (55.0, "stuvwxyz", true),  // overrun 47.8 pt, the space before the word starts 17 pt inside the edge
    (65.0, "stuvwxyz", true),  // 57.8 pt, 7 pt inside
    (71.0, "stuvwxyz", true),  // 63.8 pt, 1 pt inside
    (73.0, "stuvwxyz", false), // 65.8 pt, 1 pt past the edge
    (65.0, "uvwxyz", true),    // 43.4 pt, 7 pt inside
    (71.0, "yz", true),        // 20.6 pt, 1 pt inside
    (73.0, "yz", false),       // 22.6 pt, 1 pt past the edge
];

#[test]
fn a_long_overrunning_word_stays_while_the_space_before_it_starts_inside_the_line() {
    crate::test_fonts();
    let head = format!("P00{}", " ab".repeat(19));
    for &(indent_right, word, kept) in WORD_LENGTH_PROBES {
        for align in [Align::Left, Align::Justify] {
            let line = format!("{head} {word}");
            let text = format!("{line} cd cd cd cd cd cd end");
            let chr = wordcraft_doc::CharProps { font: Some("Courier New".into()), size: Some(12.0), ..Default::default() };
            let mut para = wordcraft_doc::Paragraph::with_text(&text, chr);
            para.props.align = Some(align);
            para.props.indent_right = Some(indent_right);
            para.props.space_after = Some(0.0);
            let mut d = Document::new();
            d.last_section.page_w = 612.0;
            d.last_section.margin_left = LEFT;
            d.last_section.margin_right = 36.0;
            d.settings.rtf_justification = true;
            d.body = vec![wordcraft_doc::para_block(para)];
            let l = lay(&d);
            let (para, x) =
                l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, x, .. } = it { Some((para.clone(), *x)) } else { None }).unwrap();
            let first = &para.lines[0];
            let shown = text[first.start..first.stop].trim_end();
            let want = if kept { line.as_str() } else { head.as_str() };
            assert_eq!(shown, want, "{word} at {indent_right} pt, {align:?}: first line");
            let end = para.x_of(0, first.start + shown.len()).unwrap() + x;
            let edge = 612.0 - 36.0 - indent_right;
            assert!(end <= edge + 0.05, "{word} at {indent_right} pt, {align:?}: the line ends at {end}, past the edge {edge}");
        }
    }
}
