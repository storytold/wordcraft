use super::page::revision_color;
use super::paint::Stroke;
use super::{Document, Draw, HashMap, Rect, Rgb, ShapeKind, Spin, TextColor};

/// Each author's place among `authors` in order of first appearance (which colour marks their
/// changes and comments), found in one pass.
pub(crate) fn author_order<'a>(authors: impl Iterator<Item = &'a str>) -> HashMap<&'a str, u32> {
    let mut seen = HashMap::new();
    for a in authors {
        crate::work::steps(1);
        let n = u32::try_from(seen.len()).unwrap_or(u32::MAX);
        seen.entry(a).or_insert(n);
    }
    seen
}

/// The colours of the document's revision authors ([`author_order`]), worked out once for a
/// drawing, when its first tracked change is drawn, not once for every change.
pub(super) struct Authors<'a> {
    doc: &'a Document,
    order: std::cell::OnceCell<HashMap<&'a str, u32>>,
}

impl<'a> Authors<'a> {
    pub(super) fn new(doc: &'a Document) -> Self {
        Authors { doc, order: std::cell::OnceCell::new() }
    }

    /// The colour of revision `r`'s author.
    pub(super) fn color(&self, r: u32) -> Rgb {
        let order = self.order.get_or_init(|| author_order(self.doc.revisions.iter().map(|v| v.author.as_str())));
        revision_color(self.doc.revisions.get(r as usize).and_then(|v| order.get(v.author.as_str()).copied()).unwrap_or(0))
    }
}

/// How far a legacy check box's square sits inside the room it takes on its line, on every side.
pub const CHECK_BOX_INSET: f32 = 1.16;

/// A legacy form check box (`FORMCHECKBOX`) in `frame`, the room it takes on its line (as wide
/// as the text's line is tall): a square inset [`CHECK_BOX_INSET`] with a 0.72 pt border, and,
/// checked, an X from corner to corner in 0.48 pt lines, in the text's colour. Measured on Word's
/// PDF export: a 9.96 pt square for 10 pt Calibri, 22.08 pt for 20 pt.
pub fn check_box(frame: Rect, checked: bool, color: Rgb, alpha: f32, out: &mut Vec<Draw>) {
    let i = CHECK_BOX_INSET.min(frame.w / 4.0).min(frame.h / 4.0).max(0.0);
    let sq = Rect::new(frame.x + i, frame.y + i, (frame.w - 2.0 * i).max(0.5), (frame.h - 2.0 * i).max(0.5));
    let line =
        |x0: f32, y0: f32, x1: f32, y1: f32, width: f32| Draw::Line { x0, y0, x1, y1, width, color, stroke: Stroke::Solid, alpha, text_bg: None };
    let (l, t, r, b) = (sq.x, sq.y, sq.x + sq.w, sq.y + sq.h);
    out.push(Draw::Shape {
        rect: sq,
        kind: ShapeKind::Rectangle,
        fill: None,
        stroke: Some(color),
        stroke_width: 0.72,
        effects: Default::default(),
        corner: None,
        dash: Vec::new(),
        cap: Default::default(),
    });
    if checked {
        out.extend([line(l, t, r, b, 0.48), line(r, t, l, b, 0.48)]);
    }
}

/// Automatic colour is black, or white on a dark background.
pub fn text_color(c: &TextColor, background: Option<Rgb>) -> Rgb {
    match c {
        TextColor::Rgb(r) => *r,
        TextColor::Auto => {
            if background.is_some_and(|b| b.luma() < 100.0) {
                Rgb::WHITE
            } else {
                Rgb::BLACK
            }
        }
    }
}

/// The characters that stand upright in top-to-bottom text, as sorted, disjoint, inclusive ranges
/// of code points: those whose Unicode vertical orientation (UAX #50, `VerticalOrientation.txt` of
/// Unicode 18.0.0) is `U`, upright, or `Tu`, a vertical form where the font has one and otherwise
/// upright (no vertical forms are substituted here, so they stand as they are). `R` and `Tr`
/// characters (Latin, digits, arrows, brackets, dashes, the prolonged sound mark) turn with the line.
/// Two groups the property marks `U` are left out and turn with the line:
/// - the Latin-1 signs § © ® ± ¼ ½ ¾ × ÷: in mixed text they are usually set in the Latin font,
///   where upright they look small and out of place among the turned letters around them;
/// - private use (U+E000–F8FF and planes 15 and 16): a document's private-use characters are
///   mostly symbol-font codes (U+F020–F0FF), whose glyphs differ from one font to the next.
const UPRIGHT: [(u32, u32); 93] = [
    (0x02EA, 0x02EB),   // Spacing Modifier Letters
    (0x1100, 0x11FF),   // Hangul Jamo
    (0x1401, 0x167F),   // Unified Canadian Aboriginal Syllabics
    (0x18B0, 0x18FF),   // Unified Canadian Aboriginal Syllabics Extended
    (0x2016, 0x2016),   // General Punctuation
    (0x2020, 0x2021),   // General Punctuation
    (0x2030, 0x2031),   // General Punctuation
    (0x203B, 0x203C),   // General Punctuation
    (0x2042, 0x2042),   // General Punctuation
    (0x2047, 0x2049),   // General Punctuation
    (0x2051, 0x2051),   // General Punctuation
    (0x2065, 0x2065),   // General Punctuation
    (0x20DD, 0x20E0),   // Combining Diacritical Marks for Symbols
    (0x20E2, 0x20E4),   // Combining Diacritical Marks for Symbols
    (0x2100, 0x2101),   // Letterlike Symbols
    (0x2103, 0x2109),   // Letterlike Symbols
    (0x210F, 0x210F),   // Letterlike Symbols
    (0x2113, 0x2114),   // Letterlike Symbols
    (0x2116, 0x2117),   // Letterlike Symbols
    (0x211E, 0x2123),   // Letterlike Symbols
    (0x2125, 0x2125),   // Letterlike Symbols
    (0x2127, 0x2127),   // Letterlike Symbols
    (0x2129, 0x2129),   // Letterlike Symbols
    (0x212E, 0x212E),   // Letterlike Symbols
    (0x2135, 0x213F),   // Letterlike Symbols
    (0x2145, 0x214A),   // Letterlike Symbols
    (0x214C, 0x214D),   // Letterlike Symbols
    (0x214F, 0x2189),   // Letterlike Symbols to Number Forms
    (0x218C, 0x218F),   // Number Forms
    (0x221E, 0x221E),   // Mathematical Operators
    (0x2234, 0x2235),   // Mathematical Operators
    (0x2300, 0x2307),   // Miscellaneous Technical
    (0x230C, 0x231F),   // Miscellaneous Technical
    (0x2324, 0x2328),   // Miscellaneous Technical
    (0x232B, 0x232B),   // Miscellaneous Technical
    (0x237D, 0x239A),   // Miscellaneous Technical
    (0x23BE, 0x23CD),   // Miscellaneous Technical
    (0x23CF, 0x23CF),   // Miscellaneous Technical
    (0x23D1, 0x23DB),   // Miscellaneous Technical
    (0x23E2, 0x2422),   // Miscellaneous Technical to Control Pictures
    (0x2424, 0x24FF),   // Control Pictures to Enclosed Alphanumerics
    (0x25A0, 0x2619),   // Geometric Shapes to Miscellaneous Symbols
    (0x2620, 0x2767),   // Miscellaneous Symbols to Dingbats
    (0x2776, 0x2793),   // Dingbats
    (0x2B12, 0x2B2F),   // Miscellaneous Symbols and Arrows
    (0x2B50, 0x2B59),   // Miscellaneous Symbols and Arrows
    (0x2B97, 0x2B97),   // Miscellaneous Symbols and Arrows
    (0x2BB8, 0x2BD1),   // Miscellaneous Symbols and Arrows
    (0x2BD3, 0x2BEB),   // Miscellaneous Symbols and Arrows
    (0x2BF0, 0x2BFF),   // Miscellaneous Symbols and Arrows
    (0x2E50, 0x2E51),   // Supplemental Punctuation
    (0x2E80, 0x3007),   // CJK Radicals Supplement to CJK Symbols and Punctuation
    (0x3012, 0x3013),   // CJK Symbols and Punctuation
    (0x3020, 0x302F),   // CJK Symbols and Punctuation
    (0x3031, 0x309F),   // CJK Symbols and Punctuation to Hiragana
    (0x30A1, 0x30FB),   // Katakana
    (0x30FD, 0xA4CF),   // Katakana to Yi Radicals
    (0xA960, 0xA97F),   // Hangul Jamo Extended-A
    (0xAC00, 0xD7FF),   // Hangul Syllables to Hangul Jamo Extended-B
    (0xF900, 0xFAFF),   // CJK Compatibility Ideographs
    (0xFE10, 0xFE1F),   // Vertical Forms
    (0xFE30, 0xFE48),   // CJK Compatibility Forms
    (0xFE50, 0xFE57),   // Small Form Variants
    (0xFE5F, 0xFE62),   // Small Form Variants
    (0xFE67, 0xFE6F),   // Small Form Variants
    (0xFF01, 0xFF07),   // Halfwidth and Fullwidth Forms
    (0xFF0A, 0xFF0C),   // Halfwidth and Fullwidth Forms
    (0xFF0E, 0xFF19),   // Halfwidth and Fullwidth Forms
    (0xFF1F, 0xFF3A),   // Halfwidth and Fullwidth Forms
    (0xFF3C, 0xFF3C),   // Halfwidth and Fullwidth Forms
    (0xFF3E, 0xFF3E),   // Halfwidth and Fullwidth Forms
    (0xFF40, 0xFF5A),   // Halfwidth and Fullwidth Forms
    (0xFFE0, 0xFFE2),   // Halfwidth and Fullwidth Forms
    (0xFFE4, 0xFFE7),   // Halfwidth and Fullwidth Forms
    (0xFFF0, 0xFFF8),   // Specials
    (0xFFFC, 0xFFFD),   // Specials
    (0x10980, 0x1099F), // Meroitic Hieroglyphs
    (0x11580, 0x115FF), // Siddham
    (0x11A00, 0x11ABF), // Zanabazar Square to Unified Canadian Aboriginal Syllabics Extended-A
    (0x13000, 0x1467F), // Egyptian Hieroglyphs to Anatolian Hieroglyphs
    (0x16FE0, 0x191DF), // Ideographic Symbols and Punctuation to Jurchen Radicals
    (0x1AFF0, 0x1B2FF), // Kana Extended-B to Nushu
    (0x1CEC0, 0x1CEDC), // Miscellaneous Symbols Supplement
    (0x1CEE0, 0x1CEF0), // Miscellaneous Symbols Supplement
    (0x1CEFE, 0x1CFCF), // Miscellaneous Symbols Supplement to Znamenny Musical Notation
    (0x1D000, 0x1D1FF), // Byzantine Musical Symbols to Musical Symbols
    (0x1D250, 0x1D281), // Musical Symbols Supplement
    (0x1D2E0, 0x1D37F), // Mayan Numerals to Counting Rod Numerals
    (0x1D800, 0x1DAAF), // Sutton SignWriting
    (0x1F000, 0x1F7FF), // Mahjong Tiles to Geometric Shapes Extended
    (0x1F900, 0x1FAFF), // Supplemental Symbols and Pictographs to Symbols and Pictographs Extended-A
    (0x20000, 0x2FFFD), // Supplementary Ideographic Plane
    (0x30000, 0x3FFFD), // Tertiary Ideographic Plane
];

/// Whether `c` stands upright in top-to-bottom East Asian text (`tbRl`) rather than turning with
/// the line: ideographs, kana, Hangul, full-width letters and digits, the ideographic comma and
/// full stop, and symbols such as ▲ ★ ※ ① Ⅳ ℃ (the table `UPRIGHT`).
pub fn upright_in_vertical(c: char) -> bool {
    let c = u32::from(c);
    let i = UPRIGHT.partition_point(|&(_, last)| last < c);
    UPRIGHT.get(i).is_some_and(|&(first, _)| first <= c)
}

/// Top-to-bottom text drawn turned 90° clockwise, with its upright characters
/// ([`upright_in_vertical`]) turned back, each about the centre of its em square (`tbRl`:
/// ideographs and symbols such as ▲ stand, Latin lies).
/// Glyph runs are split where uprightness changes, so the text keeps its order.
pub(super) fn upright_east_asian(items: Vec<Draw>) -> Vec<Draw> {
    let mut out = Vec::with_capacity(items.len());
    for d in items {
        let Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, text_bg, x_scale } = d else {
            out.push(d);
            continue;
        };
        let up = |k: usize| ranges.get(k).and_then(|r| text.get(r.clone())).and_then(|t| t.chars().next()).is_some_and(upright_in_vertical);
        if ranges.len() != glyphs.len() || !(0..glyphs.len()).any(up) {
            out.push(Draw::Glyphs { face, size, glyphs, color, alpha, synth_bold, synth_italic, text, link, ranges, text_bg, x_scale });
            continue;
        }
        let piece = |k0: usize, k1: usize| -> Draw {
            let gs = glyphs.get(k0..k1).unwrap_or(&[]).to_vec();
            let rs = ranges.get(k0..k1).unwrap_or(&[]);
            let (b0, b1) = (rs.iter().map(|r| r.start).min().unwrap_or(0), rs.iter().map(|r| r.end).max().unwrap_or(0));
            let t = text.get(b0..b1).unwrap_or("").to_string();
            let rs = rs.iter().map(|r| r.start.saturating_sub(b0)..r.end.saturating_sub(b0)).collect();
            Draw::Glyphs { face, size, glyphs: gs, color, alpha, synth_bold, synth_italic, text: t, link: link.clone(), ranges: rs, text_bg, x_scale }
        };
        let mut k0 = 0;
        while k0 < glyphs.len() {
            let u = up(k0);
            let k1 = (k0 + 1..glyphs.len()).find(|&k| up(k) != u).unwrap_or(glyphs.len());
            if !u {
                out.push(piece(k0, k1));
            } else {
                for k in k0..k1 {
                    let Some(&(_, gx, base)) = glyphs.get(k) else { continue };
                    // The em square: one em along the line (the advance to the next glyph when
                    // known), from about 0.88 em above the baseline to 0.12 em below it.
                    let adv = glyphs.get(k + 1).map(|g| g.1 - gx).filter(|a| *a > 0.0 && *a < size * 2.0).unwrap_or(size);
                    let (cx, cy) = (gx + adv / 2.0, base - size * 0.38);
                    let mut g = piece(k, k + 1);
                    // The ideographic comma and full stop sit in the upper right of their square in
                    // vertical text (lower left in horizontal): moved there before standing up.
                    if let Draw::Glyphs { text, glyphs, .. } = &mut g
                        && matches!(text.chars().next(), Some('\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}'))
                    {
                        for (_, x, y) in glyphs.iter_mut() {
                            *x += size * 0.55;
                            *y -= size * 0.55;
                        }
                    }
                    out.push(Draw::Rotated { cx, cy, spin: Spin::new(-90.0, false, false), items: vec![g] });
                }
            }
            k0 = k1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::UPRIGHT;

    /// `upright_in_vertical` searches the ranges by halves: they must be sorted, with a gap between
    /// each and the next (adjacent ones would be one range), and each within Unicode.
    #[test]
    fn upright_ranges_are_sorted_and_apart() {
        assert!(UPRIGHT.iter().all(|&(first, last)| first <= last && last <= 0x10FFFF));
        for w in UPRIGHT.windows(2) {
            let &[(_, last), (next, _)] = w else { continue };
            assert!(last + 1 < next, "{last:04X} then {next:04X}");
        }
    }
}
