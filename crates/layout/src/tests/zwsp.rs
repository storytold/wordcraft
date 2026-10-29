//! A zero-width space (U+200B) stays in its run's font even when the font has no glyph for it:
//! Word sizes a line holding only zero-width spaces like one of text in that font, instead of in
//! a fallback face with taller line metrics (measured on a table-heavy report: a 10pt Aptos cell
//! holding only U+200B is as tall as its neighbours' text, 12.2pt, not the 13.3pt of a fallback).

use crate::{Document, LayoutCache, LayoutOptions, Placed, layout};

const FAMILY: &str = "Nozws";

/// Inter without a U+200B mapping and with short (1600/400 of 2048) line metrics, under the family
/// name "Nozws". Built from the bundled font so the test needs no installed font; every face
/// that could stand in for U+200B has taller lines.
fn no_zwsp_family() -> &'static str {
    static ADDED: std::sync::Once = std::sync::Once::new();
    ADDED.call_once(|| {
        let inter = wordcraft_fonts::word::resolve("Inter", false, false).face;
        assert!(inter.covers('\u{200b}'), "Inter maps U+200B");
        let mut data = inter.data().to_vec();
        let u16_at = |d: &[u8], o: usize| u16::from_be_bytes([d[o], d[o + 1]]) as usize;
        let u32_at = |d: &[u8], o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
        let table = |d: &[u8], tag: &[u8; 4]| {
            (0..u16_at(d, 4)).map(|i| 12 + 16 * i).find(|&r| &d[r..r + 4] == tag).map(|r| (u32_at(d, r + 8), u32_at(d, r + 12))).unwrap()
        };
        // Inter's U+200B ends its character map segment (U+2000..U+200B) in every subtable: end
        // the segment one code point earlier.
        let (cmap, _) = table(&data, b"cmap");
        let mut ended = 0;
        for i in 0..u16_at(&data, cmap + 2) {
            let sub = cmap + u32_at(&data, cmap + 4 + 8 * i + 4);
            match u16_at(&data, sub) {
                4 => {
                    let segs = u16_at(&data, sub + 6) / 2;
                    for s in 0..segs {
                        let end = sub + 14 + 2 * s;
                        if u16_at(&data, end) == 0x200b {
                            data[end..end + 2].copy_from_slice(&0x200a_u16.to_be_bytes());
                            ended += 1;
                        }
                    }
                }
                12 => {
                    for g in 0..u32_at(&data, sub + 12) {
                        let end = sub + 16 + 12 * g + 4;
                        if u32_at(&data, end) == 0x200b {
                            data[end..end + 4].copy_from_slice(&0x200a_u32.to_be_bytes());
                            ended += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(ended > 0, "found U+200B at a segment end");
        // Short lines: hhea, typographic and Windows ascent and descent.
        let (hhea, _) = table(&data, b"hhea");
        data[hhea + 4..hhea + 10].copy_from_slice(&[1600_i16.to_be_bytes(), (-400_i16).to_be_bytes(), 0_i16.to_be_bytes()].concat());
        let (os2, _) = table(&data, b"OS/2");
        data[os2 + 68..os2 + 78].copy_from_slice(
            &[1600_i16.to_be_bytes(), (-400_i16).to_be_bytes(), 0_i16.to_be_bytes(), 1600_u16.to_be_bytes(), 400_u16.to_be_bytes()].concat(),
        );
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
    });
    FAMILY
}

fn chr() -> wordcraft_doc::CharProps {
    wordcraft_doc::CharProps { font: Some(no_zwsp_family().into()), size: Some(10.0), ..Default::default() }
}

fn para(text: &str) -> wordcraft_doc::Paragraph {
    let mut p = wordcraft_doc::Paragraph::with_text(text, chr());
    p.mark = chr();
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
    p
}

/// The heights of the first line of each body paragraph and table cell paragraph, in order.
fn line_heights(d: &Document) -> Vec<f32> {
    let mut c = LayoutCache::new();
    let l = layout(d, &mut c, &LayoutOptions::default());
    l.pages[0].items.iter().filter_map(|it| if let Placed::Lines { para, .. } = it { para.lines.first().map(|l| l.height) } else { None }).collect()
}

#[test]
fn the_test_font_lacks_a_zero_width_space_and_has_short_lines() {
    let face = wordcraft_fonts::word::resolve(no_zwsp_family(), false, false).face;
    assert_eq!(face.family, FAMILY, "the test font is installed under its own name");
    assert!(!face.covers('\u{200b}'));
    let (a, d) = wordcraft_fonts::word::line_metrics(&face);
    assert!(((a + d) / face.upem - 2000.0 / 2048.0).abs() < 1e-6, "{a} {d}");
}

#[test]
fn a_line_of_zero_width_spaces_is_as_tall_as_a_line_of_text() {
    let mut d = Document::new();
    d.body = ["x", "\u{200b}", "\u{200b}\u{200b}", "a\u{200b}b"].into_iter().map(|t| wordcraft_doc::para_block(para(t))).collect();
    let h = line_heights(&d);
    assert_eq!(h.len(), 4, "{h:?}");
    // 10pt in a 2000/2048 em line.
    assert!((h[0] - 10.0 * 2000.0 / 2048.0).abs() < 0.01, "{h:?}");
    for (i, x) in h.iter().enumerate() {
        assert!((x - h[0]).abs() < 0.01, "line {i}: {h:?}");
    }
}

/// The case found in the report: a table row whose cells hold only zero-width spaces is as tall
/// as a row of text in the same font.
#[test]
fn a_row_of_zero_width_spaces_is_as_tall_as_a_row_of_text() {
    let mut d = Document::new();
    let mut t = wordcraft_doc::Table::new(3, 2, 200.0);
    t.props.cell_margins = Some([0.0, 5.4, 0.0, 5.4]);
    for (row, text) in t.rows.iter_mut().zip(["Net", "\u{200b}", "Cost"]) {
        for cell in &mut row.cells {
            cell.blocks = vec![wordcraft_doc::para_block(para(text))];
        }
    }
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t)), wordcraft_doc::para_block(para("after"))];
    let mut c = LayoutCache::new();
    let l = layout(&d, &mut c, &LayoutOptions::default());
    let tops: Vec<f32> = l.pages[0].items.iter().filter_map(|it| if let Placed::Lines { y, .. } = it { Some(*y) } else { None }).collect();
    let mut rows: Vec<f32> = tops.clone();
    rows.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    assert_eq!(rows.len(), 4, "{tops:?}");
    let steps: Vec<f32> = rows.windows(2).map(|w| w[1] - w[0]).collect();
    assert!((steps[1] - steps[0]).abs() < 0.01 && (steps[2] - steps[0]).abs() < 0.01, "row steps {steps:?}");
}

/// A field result that starts in the run's short ASCII font and goes on in its taller high-ANSI
/// font (`w:hAnsi`) is one cluster: its line is as tall as the taller font's, as it would be for
/// the same text outside a field.
#[test]
fn a_field_result_partly_in_a_taller_high_ansi_font_makes_its_line_taller() {
    let with_field = |result: &str| {
        let hansi = wordcraft_doc::CharProps { font_hansi: Some("Source Sans 3".into()), ..chr() };
        let mut d = Document::new();
        let mut p = para("p ");
        p.mark = hansi.clone();
        d.body = vec![wordcraft_doc::para_block(p)];
        let field = wordcraft_doc::InlineObject::Field {
            instr: "DOCPROPERTY Title".into(),
            result: result.into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        };
        d.insert_object(&wordcraft_doc::Pos::body(0, 2), field, &hansi).unwrap();
        d
    };
    let text = |t: &str| {
        let hansi = wordcraft_doc::CharProps { font_hansi: Some("Source Sans 3".into()), ..chr() };
        let mut d = Document::new();
        let mut p = wordcraft_doc::Paragraph::with_text(t, hansi.clone());
        p.mark = hansi;
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(0.0);
        p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
        d.body = vec![wordcraft_doc::para_block(p)];
        d
    };
    let short = line_heights(&with_field("A"))[0];
    let mixed = line_heights(&with_field("A\u{e9}"))[0];
    let as_text = line_heights(&text("p A\u{e9}"))[0];
    assert!(mixed > short + 0.5, "the é makes the line taller: {short} {mixed}");
    assert!((mixed - as_text).abs() < 0.01, "as tall as the same text outside a field: {mixed} {as_text}");
}

/// A run with a distinct high-ANSI font (`w:hAnsi`, here one with taller lines): a zero-width
/// space is not high-ANSI text, so it stays in the run's ASCII font, at its line height. It
/// counted as non-ASCII Latin text, and as a mark it also took the high-ANSI choice of the
/// character before it.
#[test]
fn a_zero_width_space_stays_out_of_a_distinct_high_ansi_font() {
    let with_hansi = |t: &str| {
        let hansi = wordcraft_doc::CharProps { font_hansi: Some("Source Sans 3".into()), ..chr() };
        let mut p = wordcraft_doc::Paragraph::with_text(t, hansi.clone());
        p.mark = hansi;
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(0.0);
        p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
        p
    };
    let mut d = Document::new();
    d.body = ["x", "\u{200b}", "\u{200b}\u{200b}"].into_iter().map(|t| wordcraft_doc::para_block(with_hansi(t))).collect();
    let h = line_heights(&d);
    assert_eq!(h.len(), 3, "{h:?}");
    assert!(h.iter().all(|v| (v - h[0]).abs() < 0.01), "lines of zero-width spaces as tall as text in the run's font: {h:?}");
}
