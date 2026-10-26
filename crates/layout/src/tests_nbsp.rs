//! A no-break space is laid out as wide as an ordinary space, whatever the font's own U+00A0
//! glyph measures (Lucida Bright draws it twice as wide).

use super::*;

/// Inter with its U+00A0 glyph twice as wide as its space, under the family name "Wnbsp".
/// Built from the bundled font so the test needs no installed font.
fn wide_nbsp_family() -> &'static str {
    const FAMILY: &str = "Wnbsp";
    let inter = wordcraft_fonts::word::resolve("Inter", false, false).face;
    let mut data = inter.data().to_vec();
    let gid = inter.glyph_for('\u{a0}') as usize;
    assert_ne!(gid, inter.glyph_for(' ') as usize, "Inter keeps a glyph of its own for U+00A0");
    let u16_at = |d: &[u8], o: usize| u16::from_be_bytes([d[o], d[o + 1]]) as usize;
    let u32_at = |d: &[u8], o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
    let table = |d: &[u8], tag: &[u8; 4]| {
        (0..u16_at(d, 4)).map(|i| 12 + 16 * i).find(|&r| &d[r..r + 4] == tag).map(|r| (u32_at(d, r + 8), u32_at(d, r + 12))).unwrap()
    };
    let (hmtx, _) = table(&data, b"hmtx");
    assert!(gid < u16_at(&data, table(&data, b"hhea").0 + 34), "the glyph has a full metrics record");
    let advance = u16_at(&data, hmtx + 4 * gid);
    data[hmtx + 4 * gid..hmtx + 4 * gid + 2].copy_from_slice(&((advance * 2) as u16).to_be_bytes());
    // The family name, in the name table's UTF-16 and single-byte records alike.
    let (name, len) = table(&data, b"name");
    let wide = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(wide("Inter"), wide(FAMILY)), (b"Inter".to_vec(), FAMILY.as_bytes().to_vec())] {
        for i in name..name + len - from.len() {
            if data[i..i + from.len()] == from[..] {
                data[i..i + from.len()].copy_from_slice(&to);
            }
        }
    }
    wordcraft_fonts::FontDb::global().add_font(data);
    FAMILY
}

/// The width of the first line of `text` set in the wide-nbsp font.
fn line_width(text: &str) -> f32 {
    let chr = wordcraft_doc::CharProps { font: Some(wide_nbsp_family().into()), size: Some(12.0), ..Default::default() };
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, chr))];
    let mut c = LayoutCache::new();
    let l = layout(&d, &mut c, &LayoutOptions::default());
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { para, .. } = it {
                let xs = &para.lines[0].xs;
                return xs[xs.len() - 1] - xs[0];
            }
        }
    }
    panic!("paragraph not laid out");
}

#[test]
fn a_no_break_space_is_as_wide_as_a_space() {
    let face = wordcraft_fonts::word::resolve(wide_nbsp_family(), false, false).face;
    assert_eq!(face.family, wide_nbsp_family(), "the test font is installed under its own name");
    assert!(
        face.advance(face.glyph_for('\u{a0}')) > 1.9 * face.advance(face.glyph_for(' ')),
        "the font's own U+00A0 glyph is twice as wide as its space"
    );
    let nbsp = line_width(&format!("|{}|", "\u{a0}".repeat(10)));
    let space = line_width(&format!("|{}|", " ".repeat(10)));
    assert!((nbsp - space).abs() < 0.01, "ten no-break spaces measure {nbsp} pt, ten spaces {space} pt");
}
