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
//! Embeddings and overrides (`w:dir`, §17.3.2.8; `w:bdo`, §17.3.2.3) are kept in the paragraph
//! text as the Unicode explicit formatting characters; [`direction_elements`] finds the pairs
//! that are written back as the elements.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::styles::StyleSheet;
use wordcraft_doc::{InlineObject, Paragraph};

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

/// One end of a `w:dir` / `w:bdo` element: `Open(element, w:val)` or `Close(element)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DirElement {
    Open(&'static str, &'static str),
    Close(&'static str),
}

/// Nesting deeper than this stays text (the bidi algorithm stops at 125 levels anyway).
const MAX_NEST: usize = 32;

/// The explicit formatting characters of `p` (U+202A LRE, U+202B RLE, U+202D LRO, U+202E RLO
/// and their U+202C PDF) written as `w:dir` / `w:bdo` elements, by byte offset. A pair is
/// written so only when both ends are in the same hyperlink and no field, content control or
/// comment range boundary lies between them; other characters stay text.
pub(super) fn direction_elements(p: &Paragraph) -> HashMap<usize, DirElement> {
    let mut out = HashMap::new();
    if !p.text.chars().any(|c| ('\u{202A}'..='\u{202E}').contains(&c)) {
        return out;
    }
    let runs: Vec<(Range<usize>, &CharProps)> = p.run_ranges().collect();
    let mut ri = 0usize;
    // A segment ends at every boundary a pair may not cross.
    let mut seg = 0usize;
    let mut link: Option<&Option<String>> = None;
    let mut k = 0usize;
    // Open characters: (offset, element, w:val, segment, written as an element).
    let mut stack: Vec<(usize, &'static str, &'static str, usize, bool)> = Vec::new();
    for (at, c) in p.text.char_indices() {
        while runs.get(ri).is_some_and(|(r, _)| r.end <= at) {
            ri += 1;
        }
        let here = runs.get(ri).filter(|(r, _)| r.start <= at).map(|(_, props)| &props.link);
        if here.is_some() && link.is_some() && here != link {
            seg += 1;
        }
        if here.is_some() {
            link = here;
        }
        let open = match c {
            '\u{202A}' => Some(("w:dir", "ltr")),
            '\u{202B}' => Some(("w:dir", "rtl")),
            '\u{202D}' => Some(("w:bdo", "ltr")),
            '\u{202E}' => Some(("w:bdo", "rtl")),
            _ => None,
        };
        if let Some((tag, val)) = open {
            let ok = stack.iter().filter(|e| e.4).count() < MAX_NEST;
            stack.push((at, tag, val, seg, ok));
        } else if c == '\u{202C}' {
            if let Some((from, tag, val, s, true)) = stack.pop()
                && s == seg
            {
                out.insert(from, DirElement::Open(tag, val));
                out.insert(at, DirElement::Close(tag));
            }
        } else if c == wordcraft_doc::para::OBJ {
            if matches!(
                p.objects.get(k),
                Some(
                    InlineObject::FieldStart { .. }
                        | InlineObject::FieldEnd
                        | InlineObject::ControlStart { .. }
                        | InlineObject::ControlEnd
                        | InlineObject::CommentStart { .. }
                        | InlineObject::CommentEnd { .. }
                )
            ) {
                seg += 1;
            }
            k += 1;
        }
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
    fn pairs_become_elements_only_within_one_link() {
        let mut p = Paragraph::with_text("a\u{202B}b\u{202C}c\u{202E}d", CharProps::default());
        let d = direction_elements(&p);
        assert_eq!(d.get(&1), Some(&DirElement::Open("w:dir", "rtl")));
        assert_eq!(d.get(&5), Some(&DirElement::Close("w:dir")));
        assert_eq!(d.len(), 2, "the unpaired RLO stays text: {d:?}");
        // A link starting inside the pair: both characters stay text.
        p.format(5, p.text.len(), &|c| c.link = Some("#x".into())).unwrap();
        assert!(direction_elements(&p).is_empty());
    }

    #[test]
    fn deep_nesting_is_capped() {
        let text = format!("{}x{}", "\u{202B}".repeat(1000), "\u{202C}".repeat(1000));
        let d = direction_elements(&Paragraph::with_text(&text, CharProps::default()));
        assert_eq!(d.len(), 2 * MAX_NEST);
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
