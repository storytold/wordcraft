//! `\uN` surrogates at destination, cell, row and document ends, and the formatting the U+FFFD of an
//! unpaired one takes. Expected values checked in Word 16 (M365) on 2026-11-09 with these exact bodies.

use super::*;
use wordcraft_doc::StoryRef;

const R: char = '\u{FFFD}';

const EMOJI: char = '\u{1F600}';

fn doc(body: &str) -> wordcraft_doc::Document {
    let rtf = format!("{{\\rtf1\\ansi\\ansicpg1252{{\\fonttbl\\f0\\fswiss Helvetica;}}\\pard\\f0\\fs22\\uc0 {body}\n}}");
    import(rtf.as_bytes()).unwrap()
}

fn part_texts(d: &wordcraft_doc::Document, kind: wordcraft_doc::PartKind) -> Vec<String> {
    d.parts.iter().filter(|(_, p)| p.kind == kind).map(|(id, _)| d.plain_text(StoryRef::Part(*id)).trim_end().to_string()).collect()
}

/// The first body paragraph's runs as (text, bold).
fn bold_runs(d: &wordcraft_doc::Document) -> Vec<(String, bool)> {
    let p = d.body.iter().find_map(|b| b.as_para()).expect("a paragraph");
    let mut at = 0;
    let mut out: Vec<(String, bool)> = Vec::new();
    for r in &p.runs {
        let text = p.text.get(at..at + r.len).unwrap_or_default().to_string();
        at += r.len;
        let bold = r.props.bold == Some(true);
        match out.last_mut() {
            Some((t, b)) if *b == bold => t.push_str(&text),
            _ => out.push((text, bold)),
        }
    }
    out
}

#[test]
fn a_plain_group_end_or_a_formatting_control_does_not_break_a_pair() {
    // Word: "A😀B" for both.
    assert_eq!(doc("A{\\b\\u-10179}\\u-8704 B\\par").plain_text(StoryRef::Body).trim_end(), format!("A{EMOJI}B"));
    assert_eq!(doc("A\\u-10179\\b\\u-8704\\b0 B\\par").plain_text(StoryRef::Body).trim_end(), format!("A{EMOJI}B"));
}

#[test]
fn a_high_surrogate_left_in_a_footnote_stays_there() {
    // Word: the footnote reads U+FFFD, the body's lone low surrogate is its own U+FFFD; never U+1F600.
    let d = doc("A{\\footnote\\u-10179}\\u-8704 B\\par");
    assert_eq!(part_texts(&d, wordcraft_doc::PartKind::Footnote), [R.to_string()]);
    let body = d.plain_text(StoryRef::Body);
    assert!(!body.contains(EMOJI), "{body:?}");
    assert_eq!(body.matches(R).count(), 1, "{body:?}");
}

#[test]
fn a_high_surrogate_left_in_a_header_stays_there() {
    let d = doc("{\\header\\u-10179}\\u-8704 B\\par");
    assert_eq!(part_texts(&d, wordcraft_doc::PartKind::Header), [R.to_string()]);
    assert_eq!(d.plain_text(StoryRef::Body).trim_end(), format!("{R}B"));
}

#[test]
fn the_replacement_character_keeps_the_formatting_the_surrogate_was_read_in() {
    // Word draws this U+FFFD in the regular face, then "BOLD" bold.
    let d = doc("A\\u-10179\\b BOLD\\b0\\par");
    assert_eq!(bold_runs(&d), [(format!("A{R}"), false), ("BOLD".to_string(), true)]);
    let d = doc("{\\b A\\u-10179} C\\par");
    assert_eq!(bold_runs(&d), [(format!("A{R}"), true), (" C".to_string(), false)]);
}

#[test]
fn a_high_surrogate_just_before_a_row_end_adds_no_cell() {
    // Word: a one-cell row "A" (the surrogate sits on the row's end mark), then "\u{FFFD}C".
    let d = doc("\\trowd\\cellx2000 A\\cell\\u-10179\\row\\pard\\u-8704 C\\par");
    let t = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).expect("a table");
    assert_eq!(t.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>(), [1]);
    let body = d.plain_text(StoryRef::Body);
    assert!(!body.contains(EMOJI), "{body:?}");
    assert!(body.trim_end().ends_with(&format!("{R}C")), "{body:?}");
}

#[test]
fn a_pair_split_by_a_cell_end_is_two_replacement_characters() {
    // Word: cell 1 "A\u{FFFD}", cell 2 "\u{FFFD}B".
    let d = doc("\\trowd\\cellx2000\\cellx4000 A\\u-10179\\cell\\u-8704 B\\cell\\row\\pard C\\par");
    let t = d.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t) } else { None }).expect("a table");
    let cells: Vec<String> = t
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<String>())
        .collect();
    assert_eq!(cells, [format!("A{R}"), format!("{R}B")]);
}

#[test]
fn a_high_surrogate_at_the_end_of_the_document_is_a_replacement_character() {
    let d = import(b"{\\rtf1\\ansi\\uc0 A\\u-10240}").unwrap();
    assert_eq!(d.plain_text(StoryRef::Body).trim_end(), format!("A{R}"));
}

#[test]
fn an_unpaired_high_surrogate_with_a_fallback_character() {
    assert_eq!(doc("\\uc1 A\\u-10179?C\\par").plain_text(StoryRef::Body).trim_end(), format!("A{R}C"));
}
