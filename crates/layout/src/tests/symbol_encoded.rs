//! A font whose character map is a symbol one (Windows platform, symbol encoding) keeps its
//! glyphs at U+F020–U+F0FF, and a document stores their 8-bit codes. Installed, it draws that text
//! itself, as Word's named symbol fonts do, whatever its name (#5965).

use crate::para::ClKind;
use crate::{Document, LayoutCache, LayoutOptions, Placed, layout};

/// Inter under the family name "Dings", its character map replaced by a symbol one that maps
/// U+F020–U+F07E to Inter's glyphs for U+0020–U+007E. Built from the bundled font so the test needs
/// no installed font.
fn symbol_encoded_family() -> &'static str {
    const FAMILY: &str = "Dings";
    let inter = wordcraft_fonts::word::resolve("Inter", false, false).face;
    let mut data = inter.data().to_vec();
    let u16_at = |d: &[u8], o: usize| u16::from_be_bytes([d[o], d[o + 1]]) as usize;
    let u32_at = |d: &[u8], o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
    let record = |d: &[u8], tag: &[u8; 4]| (0..u16_at(d, 4)).map(|i| 12 + 16 * i).find(|&r| &d[r..r + 4] == tag).unwrap();
    let be = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    // A format 4 subtable: a segment for each code, then the closing segment at U+FFFF.
    let codes: Vec<(u16, u16)> = (0x20u8..0x7F).map(|b| (0xF000 + u16::from(b), u16::try_from(inter.glyph_for(char::from(b))).unwrap())).collect();
    let segs = u16::try_from(codes.len() + 1).unwrap();
    let pow = 1u16 << (15 - segs.leading_zeros());
    let mut sub = Vec::new();
    for x in [4, 16 + 8 * segs, 0, 2 * segs, 2 * pow, 15 - segs.leading_zeros() as u16, 2 * (segs - pow)] {
        be(&mut sub, x);
    }
    codes.iter().for_each(|&(code, _)| be(&mut sub, code));
    be(&mut sub, 0xFFFF);
    be(&mut sub, 0);
    codes.iter().for_each(|&(code, _)| be(&mut sub, code));
    be(&mut sub, 0xFFFF);
    codes.iter().for_each(|&(code, gid)| be(&mut sub, gid.wrapping_sub(code)));
    be(&mut sub, 1);
    (0..segs).for_each(|_| be(&mut sub, 0));
    // One encoding record: platform 3 (Windows), encoding 0 (symbol), the subtable after it.
    let mut cmap = Vec::new();
    for x in [0, 1, 3, 0] {
        be(&mut cmap, x);
    }
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&sub);
    let r = record(&data, b"cmap");
    let (at, len) = (u32_at(&data, r + 8), u32_at(&data, r + 12));
    assert!(cmap.len() <= len, "the new character map fits where Inter's was");
    data[at..at + cmap.len()].copy_from_slice(&cmap);
    data[r + 12..r + 16].copy_from_slice(&u32::try_from(cmap.len()).unwrap().to_be_bytes());
    // The family name, in the name table's UTF-16 and single-byte records alike.
    let r = record(&data, b"name");
    let (name, len) = (u32_at(&data, r + 8), u32_at(&data, r + 12));
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

/// The family drawing each text cluster of a paragraph of `text` set in `font`, with its glyphs.
fn drawn(text: &str, font: &str) -> Vec<(String, Vec<u32>)> {
    let chr = wordcraft_doc::CharProps { font: Some(font.into()), size: Some(12.0), ..Default::default() };
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, chr))];
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { para, .. } = it {
                return para
                    .clusters
                    .iter()
                    .filter(|c| c.kind == ClKind::Text)
                    .map(|c| {
                        let glyphs = para.glyphs.get(c.g0 as usize..c.g1 as usize).unwrap_or_default();
                        (para.styles[c.style as usize].face.family.clone(), glyphs.iter().map(|g| g.gid).collect())
                    })
                    .collect();
            }
        }
    }
    panic!("paragraph not laid out");
}

#[test]
fn an_installed_symbol_encoded_font_draws_the_8_bit_codes_text_stores() {
    let family = symbol_encoded_family();
    let face = wordcraft_fonts::word::resolve(family, false, false).face;
    assert_eq!(face.family, family, "the test font is installed under its own name");
    assert!(face.covers('\u{F041}') && !face.covers('A'), "its character map lists the private-use codes only");
    // "AB" as Word stores it in a symbol font: drawn from the font's glyphs at U+F041 and U+F042,
    // the glyphs the private-use codes themselves draw, not from a fallback face.
    let want: Vec<(String, Vec<u32>)> = ['\u{F041}', '\u{F042}'].map(|c| (family.to_string(), vec![face.glyph_for(c)])).to_vec();
    assert_ne!(want[0].1, vec![0], "the font has the glyph");
    assert_eq!(drawn("\u{F041}\u{F042}", family), want);
    assert_eq!(drawn("AB", family), want);
}
