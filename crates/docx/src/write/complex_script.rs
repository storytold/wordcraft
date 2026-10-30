//! Complex-script runs for Thai text.
//!
//! WordCraft breaks Thai lines by its characters, so a run need not say it is complex script.
//! Word breaks lines inside Thai words (by its dictionary) only in runs marked `w:cs` (ECMA-376
//! Part 1 §17.3.2.7); a run without it breaks only at spaces, so a Thai paragraph wraps with wide
//! gaps, or mid-word, when the file is opened in Word. Word marks text as it is typed: Thai
//! letters and digits go in `w:cs` runs, Latin letters, Western digits and `,` `;` `?` don't,
//! and other punctuation and spaces go with the text before them. A run that doesn't say `w:cs`,
//! in a paragraph with Thai, is therefore split the same way and its Thai pieces are written
//! `w:cs`. Runs that say it, or whose styles do, are written as they are.
//!
//! Word draws a `w:cs` run with the complex-script font, size, bold and italic only (`w:rFonts
//! w:cs`, `w:szCs`, `w:bCs`, `w:iCs`; 10 pt when no `w:szCs` is set anywhere), while it draws
//! unmarked Thai with the ordinary ones. A marked piece therefore also gets the values WordCraft
//! drew it with wherever its styles set no complex-script value, so it looks the same in Word.

use std::borrow::Cow;
use std::ops::Range;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::styles::{StyleKind, StyleSheet};

/// A Thai letter, mark or digit?
fn is_thai(c: char) -> bool {
    matches!(u32::from(c), 0x0E00..=0x0E7F)
}

/// How Word marks `c` when it is typed into Thai text: complex script (`Some(true)`), not
/// (`Some(false)`), or like the text before it (`None`). Measured in Word for Windows.
fn typed_mark(c: char) -> Option<bool> {
    if is_thai(c) {
        Some(true)
    } else if c.is_alphanumeric() || matches!(c, ',' | ';' | '?') {
        Some(false)
    } else {
        None
    }
}

/// Each character of `text` (byte offset, complex script?), as Word marks it: punctuation and
/// spaces follow the text before them; at the start, spaces follow the text after them and
/// punctuation is not complex script.
fn marks(text: &str) -> Vec<(usize, bool)> {
    let mut out: Vec<(usize, bool)> = Vec::new();
    let mut prev: Option<bool> = None;
    let mut leading_spaces = Vec::new();
    for (i, c) in text.char_indices() {
        let mark = match (typed_mark(c), prev) {
            (Some(m), _) | (None, Some(m)) => m,
            (None, None) if c.is_whitespace() => {
                leading_spaces.push(out.len());
                out.push((i, false));
                continue;
            }
            (None, None) => false,
        };
        if prev.is_none() {
            for &k in &leading_spaces {
                if let Some(m) = out.get_mut(k) {
                    m.1 = mark;
                }
            }
        }
        prev = Some(mark);
        out.push((i, mark));
    }
    out
}

/// The run formatting a paragraph of style `para_style` gives its runs: the document defaults
/// and the paragraph style chain. (Resolved once per paragraph.)
pub(super) fn para_props(styles: &StyleSheet, para_style: Option<&str>) -> CharProps {
    styles.para_style_props(para_style.unwrap_or("Normal")).1
}

/// `c` over `para` (from [`para_props`]) and its character style chain, as the resolver merges
/// them.
pub(super) fn merged(styles: &StyleSheet, para: &CharProps, c: &CharProps) -> CharProps {
    let mut m = para.clone();
    if let Some(s) = c.style.as_deref() {
        for st in styles.chain(s).iter().filter(|st| matches!(st.kind, StyleKind::Character | StyleKind::Paragraph)) {
            m.overlay(&st.chr);
        }
    }
    m.overlay(&CharProps { style: None, ..c.clone() });
    m
}

/// Mark `c` complex script, keeping its text as it was drawn. Word draws a `w:cs` run with the
/// complex-script font, size, bold and italic alone, and WordCraft drew the unmarked Thai with
/// the ordinary ones where `m` (what `c`'s styles and own formatting give it) has no
/// complex-script value: those are written as complex-script values.
fn mark_cs(c: &mut CharProps, m: &CharProps) {
    c.cs = Some(true);
    if m.font_cs.is_none() {
        c.font_cs = m.font.clone();
    }
    if m.size_cs.is_none() {
        c.size_cs = Some(m.size.unwrap_or(wordcraft_doc::props::DEFAULT_FONT_SIZE));
    }
    if m.bold_cs.is_none() && m.bold == Some(true) {
        c.bold_cs = Some(true);
    }
    if m.italic_cs.is_none() && m.italic == Some(true) {
        c.italic_cs = Some(true);
    }
}

/// `spans` (byte ranges of `text` with their props) with the runs that don't say `w:cs` split
/// where Word's typed marking changes; the Thai pieces are marked by [`mark_cs`], and so is a
/// tracked formatting change's "before" copy (so the change lists nothing the author didn't do)
/// unless it or its styles already say `w:cs` or `w:rtl` (rejecting the change then restores what
/// they gave). Right-to-left runs, and runs whose styles say `w:cs`, pass through. `merged` gives a
/// run's props over its styles ([`merged`]).
pub(super) fn complex_spans<'a>(
    text: &str,
    spans: Vec<(Range<usize>, Cow<'a, CharProps>)>,
    merged: impl Fn(&CharProps) -> CharProps,
) -> Vec<(Range<usize>, Cow<'a, CharProps>)> {
    if !text.chars().any(is_thai) {
        return spans;
    }
    let marks = marks(text);
    let mark_at = |at: usize| marks.binary_search_by_key(&at, |m| m.0).ok().and_then(|k| marks.get(k)).is_some_and(|m| m.1);
    let mut out = Vec::with_capacity(spans.len());
    for (range, props) in spans {
        let m = merged(&props);
        let open = m.cs.is_none() && m.rtl != Some(true);
        let Some(run) = text.get(range.clone()).filter(|_| open) else {
            out.push((range, props));
            continue;
        };
        // Where each piece starts, and whether it is complex script.
        let mut cut: Vec<(usize, bool)> = Vec::new();
        for (i, _) in run.char_indices() {
            let at = range.start.saturating_add(i);
            let cs = mark_at(at);
            if cut.last().map(|c| c.1) != Some(cs) {
                cut.push((at, cs));
            }
        }
        if !cut.iter().any(|(_, cs)| *cs) {
            out.push((range, props));
            continue;
        }
        for (k, &(start, cs)) in cut.iter().enumerate() {
            let r = start..cut.get(k + 1).map_or(range.end, |c| c.0);
            if cs {
                let mut p = props.clone().into_owned();
                mark_cs(&mut p, &m);
                if let Some(ch) = p.fmt_change.as_mut() {
                    let old = merged(&ch.old);
                    if old.cs.is_none() && old.rtl != Some(true) {
                        mark_cs(&mut ch.old, &old);
                    }
                }
                out.push((r, Cow::Owned(p)));
            } else {
                out.push((r, props.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` split by [`complex_spans`] as one plain run, written `[cs]{not}` like the probe.
    fn split(text: &str) -> String {
        let p = CharProps::default();
        let spans = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], CharProps::clone);
        spans
            .iter()
            .map(|(r, c)| {
                let t = &text[r.clone()];
                if c.cs == Some(true) { format!("[{t}]") } else { format!("{{{t}}}") }
            })
            .collect()
    }

    #[test]
    fn thai_is_split_as_word_marks_typed_text() {
        // Word for Windows, text typed through its object model and saved.
        for (text, word) in [
            ("ภาษาไทยล้วน", "[ภาษาไทยล้วน]"),
            ("ไทย ปน English 123 และ ๑๒๓ ด้วย", "[ไทย ปน ]{English 123 }[และ ๑๒๓ ด้วย]"),
            ("วันที่ 29 ตุลาคม 2569 (ทดสอบ), \"อ้างอิง\".", "[วันที่ ]{29 }[ตุลาคม ]{2569 (}[ทดสอบ)]{, \"}[อ้างอิง\".]"),
            ("ก, ข", "[ก]{, }[ข]"),
            ("ก ,ข", "[ก ]{,}[ข]"),
            ("(ก)", "{(}[ก)]"),
            ("ก (a)", "[ก (]{a)}"),
            ("ก1ข", "[ก]{1}[ข]"),
            (" ก", "[ ก]"),
            ("\"ก\"", "{\"}[ก\"]"),
            ("ก) ข", "[ก) ข]"),
            ("ก?", "[ก]{?}"),
            ("ก; ข", "[ก]{; }[ข]"),
            ("ก: ข", "[ก: ข]"),
            ("ก ๆ ข", "[ก ๆ ข]"),
            ("1ก", "{1}[ก]"),
            ("ก 1.5", "[ก ]{1.5}"),
        ] {
            assert_eq!(split(text), word, "{text}");
        }
    }

    #[test]
    fn runs_that_say_cs_or_have_no_thai_are_unchanged() {
        let text = "English only 2026";
        let p = CharProps::default();
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], CharProps::clone);
        assert!(matches!(out.as_slice(), [(r, Cow::Borrowed(_))] if *r == (0..text.len())));

        let text = "ไทย English";
        let off = CharProps { cs: Some(false), ..CharProps::default() };
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&off))], CharProps::clone);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.cs, Some(false));

        let plain = CharProps::default();
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&plain))], |c| CharProps { cs: Some(false), ..c.clone() });
        assert_eq!((out.len(), out[0].1.cs), (1, None), "a style says it");
    }

    #[test]
    fn a_tracked_change_keeps_the_flag_its_old_style_gives() {
        // The run's character style changed from one that says `w:cs` to one that doesn't: the
        // run is marked, the "before" copy keeps its style's value.
        let text = "ไทย";
        let old = CharProps { style: Some("SaysCs".into()), ..CharProps::default() };
        let p = CharProps { style: Some("Plain".into()), fmt_change: wordcraft_doc::props::PropChange::boxed(0, old), ..CharProps::default() };
        let styled = |c: &CharProps| CharProps { cs: (c.style.as_deref() == Some("SaysCs")).then_some(true), ..c.clone() };
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], styled);
        assert_eq!(out[0].1.cs, Some(true));
        assert_eq!(out[0].1.fmt_change.as_ref().unwrap().old.cs, None);
        // A right-to-left "before" copy is left as it was too.
        let rtl = CharProps { rtl: Some(true), ..CharProps::default() };
        let p = CharProps { rtl: Some(false), fmt_change: wordcraft_doc::props::PropChange::boxed(0, rtl.clone()), ..CharProps::default() };
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], CharProps::clone);
        assert_eq!(out[0].1.cs, Some(true));
        assert_eq!(out[0].1.fmt_change.as_ref().unwrap().old, rtl);
        // Without a style that says it, the "before" copy is marked too.
        let p = CharProps { fmt_change: wordcraft_doc::props::PropChange::boxed(0, CharProps::default()), ..CharProps::default() };
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], CharProps::clone);
        assert_eq!(out[0].1.fmt_change.as_ref().unwrap().old.cs, Some(true));
    }

    #[test]
    fn marked_pieces_keep_the_font_size_bold_and_italic_they_were_drawn_with() {
        // Word draws a `w:cs` run with the complex-script values alone (10 pt without `w:szCs`):
        // the ordinary ones are written as complex-script values where the styles set none.
        let text = "ไทย ABC";
        let p = CharProps { font: Some("Leelawadee UI".into()), size: Some(16.0), bold: Some(true), italic: Some(true), ..CharProps::default() };
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], CharProps::clone);
        let c = &out[0].1;
        assert_eq!(
            (c.cs, c.font_cs.as_deref(), c.size_cs, c.bold_cs, c.italic_cs),
            (Some(true), Some("Leelawadee UI"), Some(16.0), Some(true), Some(true))
        );
        assert_eq!((out[1].1.font_cs.as_deref(), out[1].1.size_cs), (None, None), "the Latin piece is left as it was");
        // Complex-script values the styles give are kept, not overwritten.
        let styled = |c: &CharProps| CharProps {
            font_cs: Some("TH SarabunPSK".into()),
            size_cs: Some(18.0),
            bold_cs: Some(false),
            italic_cs: Some(false),
            ..c.clone()
        };
        let out = complex_spans(text, vec![(0..text.len(), Cow::Borrowed(&p))], styled);
        let c = &out[0].1;
        assert_eq!((c.font_cs.as_deref(), c.size_cs, c.bold_cs, c.italic_cs), (None, None, None, None));
    }

    #[test]
    fn hostile_ranges_never_panic() {
        let p = CharProps::default();
        let text = "ไทย WordCraft";
        let spans = vec![
            (0..1, Cow::Borrowed(&p)),
            (3..1000, Cow::Borrowed(&p)),
            (usize::MAX - 1..usize::MAX, Cow::Borrowed(&p)),
            (2..text.len(), Cow::Borrowed(&p)),
        ];
        let out = complex_spans(text, spans, CharProps::clone);
        assert!(out.len() >= 4);
    }
}
