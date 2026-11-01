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
//!
//! Word draws a `w:rtl` run with the complex-script font, size, bold and italic alone (`w:rFonts
//! w:cs`, `w:szCs`, `w:bCs`, `w:iCs`), while WordCraft, like Word for a run without `w:rtl`, falls
//! back to the ordinary ones where no complex-script value is set. A bold 20 pt Courier New
//! Hebrew run with no complex-script values was drawn in Word, once marked `w:rtl`, plain, in the
//! theme's complex-script font and at its default size. A marked piece therefore also gets the
//! values WordCraft drew it with wherever its styles and own formatting set no complex-script
//! value.

use std::borrow::Cow;
use std::ops::Range;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::styles::{StyleKind, StyleSheet};

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

/// Does `c`'s character style chain give it a bidi language?
pub(super) fn char_style_has_lang_bidi(styles: &StyleSheet, c: &CharProps) -> bool {
    c.style.as_deref().is_some_and(|s| styles.chain(s).iter().any(|st| st.chr.lang_bidi.is_some()))
}

/// `c` over the run formatting of its paragraph's style chain (`para`, from
/// [`StyleSheet::para_style_props`]) and its character style chain, as the resolver merges them.
pub(super) fn over_styles(styles: &StyleSheet, para: &CharProps, c: &CharProps) -> CharProps {
    let mut m = para.clone();
    if let Some(s) = c.style.as_deref() {
        for st in styles.chain(s).iter().filter(|st| matches!(st.kind, StyleKind::Character | StyleKind::Paragraph)) {
            m.overlay(&st.chr);
        }
    }
    m.overlay(&CharProps { style: None, ..c.clone() });
    m
}

/// Add `rtl` and, when given, the bidi language `lang` to `c` where it doesn't say them. Where
/// `m` (what `c`'s styles and own formatting give it) has no complex-script font, size, bold or
/// italic, the ordinary one WordCraft drew it with is written as the complex-script value, since
/// Word draws a `w:rtl` run with those alone.
fn direct(c: &mut CharProps, lang: Option<&String>, m: &CharProps) {
    c.rtl.get_or_insert(true);
    if let Some(l) = lang
        && c.lang_bidi.is_none()
    {
        c.lang_bidi = Some(l.clone());
    }
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

/// `runs` (byte ranges of `text` with their props), split where the bidi level of `text` in a
/// paragraph of direction `rtl_para` changes parity; right-to-left pieces get `rtl: Some(true)`
/// and, unless they have one or `inherits_lang` says it comes from a style, a bidi language, and
/// the complex-script values [`direct`] gives them from `merged` (a run's props over its styles,
/// [`over_styles`]).
pub(super) fn direction_spans<'a>(
    text: &str,
    rtl_para: bool,
    runs: impl Iterator<Item = (Range<usize>, &'a CharProps)>,
    inherits_lang: impl Fn(&CharProps) -> bool,
    merged: impl Fn(&CharProps) -> CharProps,
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
        let m = merged(props);
        let old = props.fmt_change.as_ref().map(|ch| merged(&ch.old));
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
            direct(&mut p, lang.as_ref(), &m);
            // A tracked formatting change's "before" copy gets the same, so the change lists
            // nothing the author didn't do.
            if let (Some(ch), Some(old)) = (p.fmt_change.as_mut(), old.as_ref()) {
                direct(&mut ch.old, lang.as_ref(), old);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_ranges_never_panic() {
        let p = CharProps::default();
        let text = "עברית WordCraft";
        let runs = vec![(0..3, &p), (3..1000, &p), (usize::MAX - 1..usize::MAX, &p), (2..text.len(), &p)];
        let out = direction_spans(text, true, runs.into_iter(), |_| false, Clone::clone);
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
