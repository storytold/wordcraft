//! A line of a document opened from RTF (`Settings::rtf_justification`, no `\wraptrsp`) keeps the
//! word that overruns its right edge, squeezing its single spaces, when the space before that word
//! starts inside the line and the overrun is less than half the width of the line's single spaces,
//! for left, right, centred and justified lines alike, and only in a fixed-pitch font: a
//! proportional font (Times New Roman) never squeezes.
//!
//! Measured in Microsoft 365 Word (16.0.20527) from synthetic RTF files: Letter page (or 792 pt
//! wide where noted), margins 72 pt left / 36 pt right, the probe's right indent, single spaces.

use super::*;
use wordcraft_doc::props::Align;
use wordcraft_doc::{CharProps, Run};

const LEFT: f32 = 72.0;
const TAIL: &str = " cd cd cd cd cd cd cd cd cd end";

struct Probe {
    name: &'static str,
    font: &'static str,
    size: f32,
    page_width: f32,
    align: Align,
    indent_right: f32,
    /// The first line's text up to and including the overrunning word.
    line: &'static str,
    kept: bool,
}

const S20: &str = "P00 ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab ab wxyz";
const S10: &str = "P00 abcde abcde abcde abcde abcde abcde abcde abcde abcde wxyz";
const S30: &str = "P00 a a a a a a a a a a a a a a a a a a a a a a a a a a a a a wxyz";
const S5: &str = "P06 abcdefghijklm abcdefghijklm abcdefghijklm abcdefghijk wxyz";
const S15: &str = "P00 ab ab ab ab ab ab ab ab ab ab ab ab ab ab wxyz";

const fn probe(name: &'static str, size: f32, page_width: f32, align: Align, indent_right: f32, line: &'static str, kept: bool) -> Probe {
    Probe { name, font: "Courier New", size, page_width, align, indent_right, line, kept }
}

#[rustfmt::skip]
const PROBES: &[Probe] = &[
    probe("s20-ql-70", 12.0, 612.0, Align::Left, 70.0, S20, true),
    probe("s20-qj-70", 12.0, 612.0, Align::Justify, 70.0, S20, true),
    probe("s20-ql-72", 12.0, 612.0, Align::Left, 72.0, S20, false),
    probe("s20-qj-72", 12.0, 612.0, Align::Justify, 72.0, S20, false),
    probe("s20-ql-90", 12.0, 612.0, Align::Left, 90.0, S20, false),
    probe("s20-qr-90", 12.0, 612.0, Align::Right, 90.0, S20, false),
    probe("s20-qc-90", 12.0, 612.0, Align::Center, 90.0, S20, false),
    probe("s20-qj-90", 12.0, 612.0, Align::Justify, 90.0, S20, false),
    probe("s20-ql-68.4", 12.0, 612.0, Align::Left, 68.4, S20, true),
    probe("s20-qr-68.4", 12.0, 612.0, Align::Right, 68.4, S20, true),
    probe("s10-ql-91", 12.0, 612.0, Align::Left, 91.0, S10, true),
    probe("s10-ql-94", 12.0, 612.0, Align::Left, 94.0, S10, false),
    probe("s30-ql-63", 12.0, 612.0, Align::Left, 63.0, S30, true),
    probe("s30-ql-65", 12.0, 612.0, Align::Left, 65.0, S30, false),
    probe("s5-ql-74", 12.0, 612.0, Align::Left, 74.0, S5, true),
    probe("s5-ql-77", 12.0, 612.0, Align::Left, 77.0, S5, false),
    probe("f18-ql-194", 18.0, 792.0, Align::Left, 194.0, S15, true),
    probe("f18-ql-198", 18.0, 792.0, Align::Left, 198.0, S15, false),
];

fn props(font: &str, size: f32) -> CharProps {
    CharProps { font: Some(font.into()), size: Some(size), ..Default::default() }
}

/// Lays `text` out as one paragraph of an RTF document; returns the first line's text (trailing
/// spaces left out) and where its last character ends.
fn first_line(font: &str, size: f32, page_width: f32, align: Align, indent_right: f32, text: &str) -> (String, f32) {
    let mut para = wordcraft_doc::Paragraph::with_text(text, props(font, size));
    para.runs = vec![Run { len: text.len(), props: props(font, size) }];
    para.props.align = Some(align);
    para.props.indent_right = Some(indent_right);
    para.props.space_after = Some(0.0);
    let mut d = Document::new();
    d.last_section.page_w = page_width;
    d.last_section.margin_left = LEFT;
    d.last_section.margin_right = 36.0;
    d.settings.rtf_justification = true;
    d.body = vec![wordcraft_doc::para_block(para)];
    let l = lay(&d);
    let (para, x) =
        l.pages[0].items.iter().find_map(|it| if let Placed::Lines { para, x, .. } = it { Some((para.clone(), *x)) } else { None }).unwrap();
    let line = &para.lines[0];
    let shown = text[line.start..line.stop].trim_end().to_string();
    let end = para.x_of(0, line.start + shown.len()).unwrap() + x;
    (shown, end)
}

#[test]
fn every_alignment_keeps_the_overrunning_word_in_courier() {
    crate::test_fonts();
    for p in PROBES {
        let text = format!("{}{TAIL}", p.line);
        let (shown, end) = first_line(p.font, p.size, p.page_width, p.align, p.indent_right, &text);
        let before = &p.line[..p.line.rfind(' ').unwrap()];
        let want = if p.kept { p.line } else { before };
        assert_eq!(shown, want, "{}: first line", p.name);
        let edge = p.page_width - 36.0 - p.indent_right;
        assert!(end <= edge + 0.05, "{}: the line ends at {end}, past the edge {edge}", p.name);
    }
}

#[test]
fn a_proportional_font_never_squeezes() {
    crate::test_fonts();
    let font = "Times New Roman";
    let text = format!("{S30}{TAIL}");
    // The natural width of the line up to the end of wxyz, from a line wide enough for all of it.
    let (shown, natural_end) = first_line(font, 12.0, 612.0, Align::Left, 0.0, S30);
    assert_eq!(shown, S30);
    let natural = natural_end - LEFT;
    for align in [Align::Left, Align::Justify] {
        // Fits: kept.
        let ri = 612.0 - 36.0 - LEFT - natural - 0.2;
        let (shown, _) = first_line(font, 12.0, 612.0, align, ri, &text);
        assert_eq!(shown, S30, "{align:?}: fits naturally");
        // Overruns by 0.125 pt (Word: wraps).
        let ri = 612.0 - 36.0 - LEFT - natural + 0.125;
        let (shown, _) = first_line(font, 12.0, 612.0, align, ri, &text);
        assert_eq!(shown, &S30[..S30.rfind(' ').unwrap()], "{align:?}: 0.125 pt overrun wraps");
    }
}
