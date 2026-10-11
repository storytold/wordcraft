//! Numeral display modes (Word › File › Options › Advanced › Numeral): Western digits,
//! Hindi digits, or context-following digits — display only, the text is untouched.

use super::*;
use wordcraft_doc::props::ParaProps;
use wordcraft_doc::{Pos, StoryRef};

fn lay_opts(text: &str, rtl_para: bool, numeral: NumeralMode) -> DocLayout {
    let mut d = Document::from_text(text);
    if rtl_para {
        let at = Pos::body(0, 0);
        d.format_paragraphs(&at, &at, &|p: &mut ParaProps| p.bidi = Some(true)).unwrap();
    }
    let mut c = LayoutCache::new();
    layout(&d, &mut c, &LayoutOptions { numeral, ..LayoutOptions::default() })
}

/// Glyph ids of the first line's clusters covering `text`, in cluster order.
fn line_gids(l: &DocLayout, text: &str) -> Vec<Vec<u32>> {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, para, .. } = it {
                let line = &para.lines[0];
                let mut out = Vec::new();
                for k in line.c0..line.c1 {
                    let cl = &para.clusters[k];
                    assert!(cl.start <= cl.end && cl.end <= text.len(), "cluster indexes the original text");
                    let gids: Vec<u32> = para.glyphs[cl.g0 as usize..cl.g1 as usize].iter().map(|g| g.gid).collect();
                    out.push(gids);
                }
                return out;
            }
        }
    }
    panic!("body paragraph not laid out");
}

#[test]
fn parse_accepts_the_four_modes_any_case() {
    assert_eq!(NumeralMode::parse("arabic"), Some(NumeralMode::Arabic));
    assert_eq!(NumeralMode::parse("hindi"), Some(NumeralMode::Hindi));
    assert_eq!(NumeralMode::parse("context"), Some(NumeralMode::Context));
    assert_eq!(NumeralMode::parse("system"), Some(NumeralMode::System));
    assert_eq!(NumeralMode::parse("Hindi"), Some(NumeralMode::Hindi));
    assert_eq!(NumeralMode::parse("CONTEXT"), Some(NumeralMode::Context));
    assert_eq!(NumeralMode::parse("western"), None);
    assert_eq!(NumeralMode::parse(""), None);
    assert_eq!(NumeralMode::parse("hindi "), None);
}

#[test]
fn map_substitutes_digits_both_ways() {
    for d in '0'..='9' {
        let hindi = char::from_u32(0x660 + (d as u32 - '0' as u32)).unwrap();
        assert_eq!(NumeralMode::Hindi.map(d, false), hindi);
        assert_eq!(NumeralMode::Hindi.map(d, true), hindi, "Hindi mode ignores direction");
        assert_eq!(NumeralMode::Arabic.map(hindi, false), d);
        assert_eq!(NumeralMode::Arabic.map(hindi, true), d, "Arabic mode converts back both ways");
        assert_eq!(NumeralMode::Arabic.map(d, false), d);
        assert_eq!(NumeralMode::Context.map(d, true), hindi);
        assert_eq!(NumeralMode::Context.map(d, false), d);
        assert_eq!(NumeralMode::Context.map(hindi, true), hindi);
        assert_eq!(NumeralMode::Context.map(hindi, false), d);
    }
    // Letters, punctuation and Persian digits are never touched.
    for c in ['A', 'ب', '.', ' ', '(', '۰', '۹'] {
        for m in [NumeralMode::Arabic, NumeralMode::Hindi, NumeralMode::Context] {
            assert_eq!(m.map(c, false), c, "{m:?} leaves {c:?} alone (ltr)");
            assert_eq!(m.map(c, true), c, "{m:?} leaves {c:?} alone (rtl)");
        }
    }
}

#[test]
fn resolve_never_returns_system() {
    assert_eq!(NumeralMode::Arabic.resolve(), NumeralMode::Arabic);
    assert_eq!(NumeralMode::Hindi.resolve(), NumeralMode::Hindi);
    assert_eq!(NumeralMode::Context.resolve(), NumeralMode::Context);
    let r = NumeralMode::System.resolve();
    assert!(r == NumeralMode::Arabic || r == NumeralMode::Hindi, "System resolves to a concrete mode");
}

#[test]
fn hindi_mode_reshapes_digits_but_keeps_offsets() {
    let text = "Year 2024!";
    let latin = lay_opts(text, false, NumeralMode::Arabic);
    let hindi = lay_opts(text, false, NumeralMode::Hindi);
    let a = line_gids(&latin, text);
    let h = line_gids(&hindi, text);
    assert_eq!(a.len(), h.len(), "same clusters in both modes");
    assert!(!a.is_empty());
    // Letter clusters draw identical glyphs; digit clusters draw different ones.
    for (n, (ga, gh)) in a.iter().zip(h.iter()).enumerate() {
        let ch = text.chars().nth(n).unwrap();
        if ch.is_ascii_digit() {
            assert_ne!(ga, gh, "digit {ch:?} reshaped in Hindi mode");
            assert!(!gh.is_empty(), "Hindi digit draws something");
        } else {
            assert_eq!(ga, gh, "{ch:?} untouched by Hindi mode");
        }
    }
}

#[test]
fn arabic_mode_converts_hindi_digits_back() {
    let text = "عام ٢٠٢٤";
    let hindi = lay_opts(text, true, NumeralMode::Hindi);
    let arabic = lay_opts(text, true, NumeralMode::Arabic);
    let h = line_gids(&hindi, text);
    let a = line_gids(&arabic, text);
    assert_eq!(h.len(), a.len());
    let mut changed = 0;
    for (n, (gh, ga)) in h.iter().zip(a.iter()).enumerate() {
        if text.chars().nth(n).unwrap() == '٢' {
            assert_ne!(gh, ga);
            changed += 1;
        }
    }
    assert!(changed > 0, "typed Hindi digits display Western in Arabic mode");
}

#[test]
fn context_mode_follows_run_direction() {
    // Right-to-left paragraph: digits behave like Hindi mode.
    let rtl = "سنة 2024";
    let ctx = line_gids(&lay_opts(rtl, true, NumeralMode::Context), rtl);
    let hin = line_gids(&lay_opts(rtl, true, NumeralMode::Hindi), rtl);
    assert_eq!(ctx, hin, "digits in RTL text show Hindi in Context mode");
    // Left-to-right paragraph: digits behave like Arabic mode.
    let ltr = "Year 2024";
    let ctx = line_gids(&lay_opts(ltr, false, NumeralMode::Context), ltr);
    let ara = line_gids(&lay_opts(ltr, false, NumeralMode::Arabic), ltr);
    assert_eq!(ctx, ara, "digits in LTR text show Western in Context mode");
    // Mixed paragraph: digits next to Arabic show Hindi even in a left-to-right paragraph.
    let mixed = "Year سنة 2024";
    let ctx = line_gids(&lay_opts(mixed, false, NumeralMode::Context), mixed);
    let hin = line_gids(&lay_opts(mixed, false, NumeralMode::Hindi), mixed);
    assert_eq!(ctx, hin, "digits by Arabic text show Hindi in Context mode");
}

#[test]
fn remap_range_maps_display_bytes_to_original() {
    use crate::para::remap_range;
    // "a5ب": '5' (1 byte) displays as '٥' (2 bytes), so display offsets shift after it.
    let orig = "a5ب";
    let disp = "a٥ب";
    let map_starts: Vec<usize> = disp.char_indices().map(|(i, _)| i).collect();
    let orig_of: Vec<usize> = orig.char_indices().map(|(i, _)| i).collect();
    let mut orig_of = orig_of;
    orig_of.push(orig.len());
    // Display "a" [0,1) → original [0,1); "٥" [1,3) → original [1,2); "ب" [3,5) → [2,4).
    assert_eq!(remap_range(&map_starts, &orig_of, orig.len(), 0, 1), (0, 1));
    assert_eq!(remap_range(&map_starts, &orig_of, orig.len(), 1, 3), (1, 2));
    assert_eq!(remap_range(&map_starts, &orig_of, orig.len(), 3, 5), (2, 4));
    assert_eq!(remap_range(&map_starts, &orig_of, orig.len(), 0, 5), (0, 4));
    // Hostile inputs never panic and stay in range.
    assert_eq!(remap_range(&[], &[], 0, 0, 0), (0, 0));
    assert_eq!(remap_range(&map_starts, &orig_of, orig.len(), 99, 200), (4, 4));
    assert_eq!(remap_range(&map_starts, &orig_of, orig.len(), 3, 1), (1, 2));
}
