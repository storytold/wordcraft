//! [`DocLayout::missing_glyphs`] (strict export, #6005): glyph id 0 in the display list, named by
//! its character, wherever the text is drawn.

use std::sync::Arc;

use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, TextAlign};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Block, Document, Path, Pos, StoryRef, Table};
use wordcraft_fonts::FontDb;

use super::{footnote, lay, page_float, text_box_at};
use crate::display::DisplayOptions;
use crate::{DocLayout, MAX_MISSING_GLYPHS, MissingGlyphs};

/// Code points in unassigned plane 4 that no face of the host covers, checked against the body
/// font and the whole fallback chain (so the test never guesses).
fn uncovered(n: usize) -> Vec<char> {
    crate::test_fonts();
    let db = FontDb::global();
    let primary = wordcraft_fonts::word::resolve(&Document::default().settings.minor_font, false, false).face;
    let out: Vec<char> = (0x40000u32..0x4FFF0)
        .filter_map(char::from_u32)
        .filter(|c| {
            !primary.covers(*c) && db.fallback_styled(*c, primary.id(), false, false).is_none() && db.fallback_for(*c, primary.id()).is_none()
        })
        .take(n)
        .collect();
    assert_eq!(out.len(), n, "the host has faces covering all of plane 4");
    out
}

fn missing(d: &Document, l: &DocLayout) -> MissingGlyphs {
    l.missing_glyphs(d, &DisplayOptions::default(), None)
}

fn chars(m: &MissingGlyphs) -> Vec<char> {
    m.chars.iter().map(|(c, _)| *c).collect()
}

fn hf_part(d: &mut Document, kind: wordcraft_doc::PartKind, text: &str) -> u32 {
    d.add_part(kind, vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))])
}

fn chart_with(d: &mut Document, label: &str) {
    let items = vec![GraphicItem::Text {
        rect: [0.0, 100.0, 216.0, 20.0],
        text: label.into(),
        size: 10.0,
        color: Rgb::BLACK,
        bold: false,
        align: TextAlign::Center,
        font: None,
    }];
    let graphic = Graphic { kind: GraphicKind::Chart, items, ..Default::default() };
    let obj = InlineObject::Graphic { w: 216.0, h: 144.0, alt: String::new(), float: Default::default(), graphic: Arc::new(graphic) };
    d.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
}

fn equation_with(d: &mut Document, linear: &str) {
    let math = wordcraft_doc::math::Math { nodes: wordcraft_doc::math::parse_linear(linear), ..Default::default() };
    let n = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().len();
    d.para_mut(StoryRef::Body, &Path::top(0))
        .unwrap()
        .insert_object(n, InlineObject::Equation { linear: linear.into(), display: false, math }, &Default::default())
        .unwrap();
}

#[test]
fn nothing_is_missing_in_plain_text() {
    let d = Document::from_text("Hello, world.");
    assert!(missing(&d, &lay(&d)).is_empty());
}

#[test]
fn a_character_no_face_covers_is_counted_once_per_occurrence_in_the_body() {
    let c = uncovered(1)[0];
    let d = Document::from_text(&format!("a{c}b{c}{c}c"));
    let m = missing(&d, &lay(&d));
    assert_eq!(m, MissingGlyphs::from_listed(vec![(c, 3)]));
}

#[test]
fn a_missing_glyph_is_found_in_a_header() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("Body");
    d.last_section.headers.default = Some(hf_part(&mut d, wordcraft_doc::PartKind::Header, &format!("H{c}{c}")));
    assert_eq!(missing(&d, &lay(&d)).chars, vec![(c, 2)]);
}

#[test]
fn a_missing_glyph_is_found_in_a_footer() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("Body");
    d.last_section.footers.default = Some(hf_part(&mut d, wordcraft_doc::PartKind::Footer, &format!("F{c}")));
    assert_eq!(missing(&d, &lay(&d)).chars, vec![(c, 1)]);
}

#[test]
fn a_missing_glyph_is_found_in_a_footnote() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("Body text.");
    footnote(&mut d, &Pos::body(0, 4), &format!("Note {c}"));
    assert_eq!(missing(&d, &lay(&d)).chars, vec![(c, 1)]);
}

#[test]
fn a_missing_glyph_is_found_in_a_text_box() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("Body text.");
    text_box_at(&mut d, &Pos::body(0, 0), &format!("Box {c}{c}{c}"), 120.0, 40.0, page_float(300.0, 300.0));
    assert_eq!(missing(&d, &lay(&d)).chars, vec![(c, 3)]);
}

#[test]
fn a_missing_glyph_is_found_in_a_table_cell() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("before");
    let mut t = Table::new(1, 2, 300.0);
    t.rows[0].cells[1].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("cell {c}"), Default::default()))];
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Table(t)).unwrap();
    assert_eq!(missing(&d, &lay(&d)).chars, vec![(c, 1)]);
}

#[test]
fn a_missing_glyph_is_found_in_a_chart_label() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("Chart");
    chart_with(&mut d, &format!("Q{c}Q{c}"));
    assert_eq!(missing(&d, &lay(&d)).chars, vec![(c, 2)]);
}

#[test]
fn a_chart_label_run_names_each_glyphs_characters() {
    use crate::display::Draw;
    let mut d = Document::from_text("Chart");
    chart_with(&mut d, "Q1 total");
    let l = lay(&d);
    fn runs(d: &Draw, out: &mut Vec<(String, usize, usize)>) {
        match d {
            Draw::Glyphs { text, glyphs, ranges, .. } if text == "Q1 total" || !ranges.is_empty() => {
                out.push((text.clone(), glyphs.len(), ranges.len()));
                assert!(ranges.iter().all(|r| text.get(r.clone()).is_some()), "ranges on character boundaries");
            }
            Draw::Figure { draws, .. } => draws.iter().for_each(|x| runs(x, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for x in crate::display::page_display(&d, &l.pages[0], &DisplayOptions::default()) {
        runs(&x, &mut found);
    }
    assert!(found.iter().any(|(t, g, r)| t == "Q1 total" && g == r), "{found:?}");
}

#[test]
fn a_missing_glyph_is_found_in_an_equation() {
    let c = uncovered(1)[0];
    let mut d = Document::from_text("Eq: ");
    equation_with(&mut d, &format!("x+{c}"));
    let m = missing(&d, &lay(&d));
    assert_eq!(m.chars, vec![(c, 1)], "{m:?}");
}

#[test]
fn joiners_and_variation_selectors_are_never_counted() {
    let d = Document::from_text("a\u{200D}b\u{200C}c\u{FE0F}d\u{FE0E}e\u{00AD}f\u{200E}g\u{2028}h\ti");
    assert!(missing(&d, &lay(&d)).is_empty());
    // A ZWJ emoji sequence: the joiners and selectors in it never appear, whatever the host's
    // emoji face covers.
    let d = Document::from_text("\u{1F469}\u{200D}\u{1F4BB} \u{2764}\u{FE0F} \u{1F44D}\u{1F3FD}");
    let m = missing(&d, &lay(&d));
    assert!(!chars(&m).iter().any(|c| matches!(c, '\u{200D}' | '\u{FE0F}' | '\u{FE0E}')), "{m:?}");
}

#[test]
fn a_selector_beside_a_missing_character_is_not_counted_with_it() {
    let c = uncovered(1)[0];
    let d = Document::from_text(&format!("{c}\u{FE0F}"));
    assert_eq!(chars(&missing(&d, &lay(&d))), vec![c]);
}

#[test]
fn the_list_is_sorted_and_capped_at_256_distinct_code_points() {
    let all = uncovered(MAX_MISSING_GLYPHS + 44);
    let text: String = all.iter().rev().collect();
    let d = Document::from_text(&text);
    let m = missing(&d, &lay(&d));
    assert!(m.truncated);
    assert_eq!(m.chars.len(), MAX_MISSING_GLYPHS);
    assert_eq!(m.total, MAX_MISSING_GLYPHS + 44, "the total counts the unlisted ones too");
    assert!(m.chars.windows(2).all(|w| w[0].0 < w[1].0), "sorted by code point");
    // Exactly the cap's worth is no truncation.
    let d = Document::from_text(&all.iter().take(MAX_MISSING_GLYPHS).collect::<String>());
    let m = missing(&d, &lay(&d));
    assert!(!m.truncated);
    assert_eq!(m.chars.len(), MAX_MISSING_GLYPHS);
    assert_eq!(m.total, MAX_MISSING_GLYPHS);
}

#[test]
fn the_page_filter_selects_pages() {
    let [x, y] = <[char; 2]>::try_from(uncovered(2)).unwrap();
    let d = Document::from_text(&format!("first {x}\n{}last {y}", "filler line\n".repeat(150)));
    let l = lay(&d);
    let last = l.pages.len() - 1;
    assert!(last >= 1, "the document runs over pages");
    let on = |r| l.missing_glyphs(&d, &DisplayOptions::default(), r);
    assert_eq!(on(None).chars, vec![(x.min(y), 1), (x.max(y), 1)]);
    assert_eq!(on(Some(0..=0)).chars, vec![(x, 1)]);
    assert_eq!(on(Some(last..=last)).chars, vec![(y, 1)]);
    assert!(on(Some(last + 1..=last + 9)).is_empty());
    #[allow(clippy::reversed_empty_ranges)]
    let backwards = Some(1..=0);
    assert!(on(backwards).is_empty());
}

#[test]
fn an_equation_run_names_each_glyphs_characters() {
    use crate::display::Draw;
    let mut d = Document::from_text("Eq: ");
    equation_with(&mut d, "x+ab");
    let l = lay(&d);
    fn runs(d: &Draw, out: &mut Vec<(String, usize, usize)>) {
        match d {
            Draw::Glyphs { text, glyphs, ranges, .. } if !text.is_empty() => {
                assert!(ranges.iter().all(|r| text.get(r.clone()).is_some()), "ranges on character boundaries");
                out.push((text.clone(), glyphs.len(), ranges.len()));
            }
            Draw::Figure { draws, .. } | Draw::Clip { items: draws, .. } | Draw::Rotated { items: draws, .. } => {
                draws.iter().for_each(|x| runs(x, out))
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for x in crate::display::page_display(&d, &l.pages[0], &DisplayOptions::default()) {
        runs(&x, &mut found);
    }
    // The equation's runs (the body text "Eq: " is a run too) each carry one range per glyph.
    // The letters are the math italic ones where the font has them, else plain.
    assert!(found.iter().any(|(t, g, _)| (t.contains('\u{1D44E}') || t == "ab") && *g == 2), "{found:?}");
    assert!(found.iter().all(|(_, g, r)| g == r), "{found:?}");
}
