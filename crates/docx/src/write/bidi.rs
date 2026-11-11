//! Run direction for mixed right-to-left and left-to-right text (#1543).
//!
//! WordCraft lays a paragraph out with the Unicode Bidirectional Algorithm (UAX #9) from its
//! text and direction alone, so a run need not say its direction. Word orders a run's characters
//! by the run's `w:rtl` (ECMA-376 Part 1 §17.3.2.30): text in a run without it is laid out left
//! to right, so Hebrew words on either side of a Latin name, and the punctuation between them,
//! change places. A run that doesn't say its direction is therefore split where the resolved
//! level changes parity, and the right-to-left pieces are written `w:rtl`, as Word writes text
//! typed in a right-to-left paragraph. Runs that say their direction, and paragraphs without
//! right-to-left characters, are written as they are.
//!
//! Word gives right-to-left text without a bidi language (`w:lang w:bidi`, §17.3.2.20) its
//! default complex-script language, Arabic, so Hebrew would proof and show as Arabic. The pieces
//! marked `w:rtl` therefore also say their language, unless the run or its styles already do.

use std::borrow::Cow;
use std::ops::Range;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::styles::StyleSheet;

/// The language Word expects for right-to-left text of `c`'s script, for the scripts that have
/// one usual language.
fn script_lang(c: char) -> Option<&'static str> {
    match u32::from(c) {
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Some("he-IL"),
        0x0600..=0x06FF | 0x0750..=0x077F | 0x0870..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Some("ar-SA"),
        0x0700..=0x074F => Some("syr-SY"),
        0x0780..=0x07BF => Some("dv-MV"),
        _ => None,
    }
}

/// The language of the first right-to-left letter of `text` with a usual language.
fn text_lang(text: &str) -> Option<&'static str> {
    text.chars().find_map(script_lang)
}

/// A language tag of a right-to-left language (`he-IL`, `yi`, `fa-IR`, `ar-EG`…)?
fn is_rtl_lang(tag: &str) -> bool {
    let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    matches!(primary.as_str(), "he" | "iw" | "yi" | "ar" | "fa" | "ur" | "ps" | "sd" | "ug" | "ckb" | "syr" | "dv" | "ku")
}

/// Does a paragraph of style `para_style` inherit a bidi language from the document defaults or
/// its style chain? (Resolved once per paragraph.)
pub(super) fn para_inherits_lang_bidi(styles: &StyleSheet, para_style: Option<&str>) -> bool {
    styles.para_style_props(para_style.unwrap_or("Normal")).1.lang_bidi.is_some()
}

/// Does `c`'s character style chain give it a bidi language?
pub(super) fn char_style_has_lang_bidi(styles: &StyleSheet, c: &CharProps) -> bool {
    c.style.as_deref().is_some_and(|s| styles.chain(s).iter().any(|st| st.chr.lang_bidi.is_some()))
}

/// Add `rtl` and, when given, the bidi language `lang` to `c` where it doesn't say them.
fn direct(c: &mut CharProps, lang: Option<&String>) {
    c.rtl.get_or_insert(true);
    if let Some(l) = lang
        && c.lang_bidi.is_none()
    {
        c.lang_bidi = Some(l.clone());
    }
}

/// `runs` (byte ranges of `text` with their props), split where the bidi level of `text` in a
/// paragraph of direction `rtl_para` changes parity; right-to-left pieces get `rtl: Some(true)`
/// and, unless they have one or `inherits_lang` says it comes from a style, a bidi language.
pub(super) fn direction_spans<'a>(
    text: &str,
    rtl_para: bool,
    runs: impl Iterator<Item = (Range<usize>, &'a CharProps)>,
    inherits_lang: impl Fn(&CharProps) -> bool,
) -> Vec<(Range<usize>, Cow<'a, CharProps>)> {
    if !wordcraft_doc::bidi::has_rtl(text) {
        return runs.map(|(r, p)| (r, Cow::Borrowed(p))).collect();
    }
    let level = if rtl_para { unicode_bidi::Level::rtl() } else { unicode_bidi::Level::ltr() };
    let info = unicode_bidi::ParagraphBidiInfo::new(text, Some(level));
    let odd = |at: usize| info.levels.get(at).is_some_and(|l| l.is_rtl());
    let para_lang = text_lang(text);
    let mut out = Vec::new();
    for (range, props) in runs {
        let Some(run) = text.get(range.clone()).filter(|_| props.rtl.is_none()) else {
            out.push((range, Cow::Borrowed(props)));
            continue;
        };
        let named = props.lang_bidi.is_some() || inherits_lang(props);
        let piece = |r: Range<usize>, rtl: bool| {
            if !rtl {
                return (r, Cow::Borrowed(props));
            }
            let lang = if named {
                None
            } else {
                let own = props.lang.clone().filter(|l| is_rtl_lang(l));
                own.or_else(|| text.get(r.clone()).and_then(text_lang).or(para_lang).map(str::to_string))
            };
            let mut p = props.clone();
            direct(&mut p, lang.as_ref());
            // A tracked formatting change's "before" copy gets the same, so the change lists
            // nothing the author didn't do.
            if let Some(ch) = p.fmt_change.as_mut() {
                direct(&mut ch.old, lang.as_ref());
            }
            (r, Cow::Owned(p))
        };
        let mut start = range.start;
        let mut cur: Option<bool> = None;
        for (i, _) in run.char_indices() {
            let at = range.start.saturating_add(i);
            let rtl = odd(at);
            match cur {
                Some(c) if c != rtl => {
                    out.push(piece(start..at, c));
                    start = at;
                }
                _ => {}
            }
            cur = Some(rtl);
        }
        out.push(piece(start..range.end, cur.unwrap_or(false)));
    }
    out
}

/// `spans` with `w:cs` written where Word would draw a run's text in other formatting than
/// WordCraft does. Word draws a run's characters with the complex-script font, size, bold and
/// italic only when the run is complex script or right to left by its own properties, its
/// paragraph style, its table style or the run defaults; a character style saying so doesn't
/// count. Anywhere else it draws them in the run's other font and `w:sz`, Thai included (measured
/// in Word for Windows). WordCraft also draws Thai, Lao and Indic characters with the
/// complex-script formatting, and counts a character style's `w:cs`. So a run that doesn't say
/// whether it is complex script or right to left, whose two formattings differ, is split where
/// the two disagree, and those pieces are written `w:cs` (or `w:cs w:val="0"`) as WordCraft draws
/// them, as Word writes Thai typed on a Thai keyboard. `table` is the run formatting the table
/// style gives the cell the paragraph is in.
pub(super) fn complex_script_spans<'a>(
    text: &str,
    spans: Vec<(Range<usize>, Cow<'a, CharProps>)>,
    styles: &StyleSheet,
    para_style: Option<&str>,
    table: Option<&CharProps>,
) -> Vec<(Range<usize>, Cow<'a, CharProps>)> {
    let mut out = Vec::with_capacity(spans.len());
    for (range, props) in spans {
        let Some(run) = text.get(range.clone()).filter(|_| props.cs.is_none() && props.rtl.is_none()) else {
            out.push((range, props));
            continue;
        };
        let rc = styles.resolve_char_in(para_style, table, &props);
        let cs = rc.complex();
        if rc.font == cs.font && rc.size == cs.size && rc.bold == cs.bold && rc.italic == cs.italic {
            out.push((range, props));
            continue;
        }
        let word = styles.resolve_char_in(para_style, table, &CharProps { style: None, ..props.as_ref().clone() });
        let word_cs = word.cs || word.rtl;
        // What a character needs written: `Some(complex)` where Word would draw it otherwise.
        // A run that is right to left by its styles stays complex script with `w:cs w:val="0"`,
        // so that isn't written.
        let need = |c: char| {
            let draws = rc.uses_complex(c);
            (draws != word_cs && (draws || !word.rtl)).then_some(draws)
        };
        if run.chars().all(|c| need(c).is_none()) {
            out.push((range, props));
            continue;
        }
        let marked = |cs: bool| {
            let mut p = props.clone().into_owned();
            p.cs = Some(cs);
            if let Some(ch) = p.fmt_change.as_mut() {
                ch.old.cs.get_or_insert(cs);
            }
            Cow::Owned(p)
        };
        let piece = |n: Option<bool>| n.map_or_else(|| props.clone(), marked);
        let mut start = range.start;
        let mut cur: Option<Option<bool>> = None;
        for (i, c) in run.char_indices() {
            let at = range.start.saturating_add(i);
            let n = need(c);
            if let Some(was) = cur
                && was != n
            {
                out.push((start..at, piece(was)));
                start = at;
            }
            cur = Some(n);
        }
        out.push((start..range.end, piece(cur.flatten())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_ranges_never_panic() {
        let p = CharProps::default();
        let text = "עברית WordCraft";
        let runs = vec![(0..3, &p), (3..1000, &p), (usize::MAX - 1..usize::MAX, &p), (2..text.len(), &p)];
        let out = direction_spans(text, true, runs.into_iter(), |_| false);
        assert_eq!(out.len(), 5, "bad ranges pass through unsplit: {out:?}");
    }

    #[test]
    fn languages_by_script() {
        assert_eq!(text_lang("(שלום"), Some("he-IL"));
        assert_eq!(text_lang("سلام"), Some("ar-SA"));
        assert_eq!(text_lang("ܫܠܡܐ"), Some("syr-SY"));
        assert_eq!(text_lang(". ,"), None);
        assert!(is_rtl_lang("he-IL") && is_rtl_lang("yi") && is_rtl_lang("FA_ir") && !is_rtl_lang("en-US") && !is_rtl_lang(""));
    }
}
