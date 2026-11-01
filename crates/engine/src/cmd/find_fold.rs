//! Find and Replace's Match Diacritics, Match Kashida and Match Alef Hamza (#4800).
//!
//! With an option off, the searched text and the Find what text are both *folded* before they
//! are compared: diacritics are dropped (`café`, `cafe` and `cafe` + U+0301 read the same), the
//! Arabic tatweel (kashida, U+0640) is dropped, and the alef forms with hamza or madda
//! (`أ إ آ ٱ`) read as a bare alef `ا`. A folded text keeps a map back to the original, so a
//! match is reported, selected and replaced over the original characters: a base letter and
//! the marks dropped after it are one unit, and a match never ends between them.
//!
//! What counts as a diacritic (dropped when Match Diacritics is off):
//! - the combining marks of Latin, Greek and Cyrillic (U+0300–036F, U+0483–0489, U+1AB0–1AFF,
//!   U+1DC0–1DFF, U+20D0–20FF, U+FE20–FE2F), whether stored after the letter or precomposed
//!   with it (`é`, `ά`, `ё`, `ệ` decompose canonically, NFD, and lose their marks);
//! - a few letters whose mark is part of the letter and has no decomposition: `đ ł ø ħ ŧ`
//!   (and capitals) read as `d l o h t`;
//! - Hebrew points (niqqud) and cantillation marks, including precomposed letters such as `שׁ`;
//! - Arabic harakat (fatha, damma, kasra, sukun, shadda, tanwin), the superscript alef and the
//!   Quranic annotation marks.
//!
//! The hamza and madda above or below an alef are Match Alef Hamza's, not diacritics, so
//! `أ` still differs from `ا` with only Match Diacritics off. Other scripts' marks (Indic vowel
//! signs, Thai tone marks…) spell different words there, so they always match exactly.

use unicode_normalization::char::decompose_canonical;

/// Which differences a search ignores. `false` everywhere is an exact search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fold {
    /// Match Diacritics is off.
    pub diacritics: bool,
    /// Match Kashida is off.
    pub kashida: bool,
    /// Match Alef Hamza is off.
    pub alef_hamza: bool,
}

impl Fold {
    /// Some difference is ignored, so the text must be folded.
    pub fn any(self) -> bool {
        self.diacritics || self.kashida || self.alef_hamza
    }
}

const TATWEEL: char = '\u{0640}';
const ALEF: char = '\u{0627}';
/// Longest canonical decomposition of one character (Unicode's is 4); longer ones stay as they are.
const MAX_DECOMP: usize = 8;

/// A mark Match Diacritics ignores.
pub fn is_diacritic(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{0483}'..='\u{0489}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}'
        // Hebrew cantillation and points (not maqaf, paseq, sof pasuq or nun hafukha).
        | '\u{0591}'..='\u{05BD}'
        | '\u{05BF}'
        | '\u{05C1}'
        | '\u{05C2}'
        | '\u{05C4}'
        | '\u{05C5}'
        | '\u{05C7}'
        // Arabic honorifics, harakat, superscript alef and Quranic marks.
        | '\u{0610}'..='\u{061A}'
        | '\u{064B}'..='\u{0652}'
        | '\u{0656}'..='\u{065F}'
        | '\u{0670}'
        | '\u{06D6}'..='\u{06DC}'
        | '\u{06DF}'..='\u{06E4}'
        | '\u{06E7}'
        | '\u{06E8}'
        | '\u{06EA}'..='\u{06ED}')
}

/// Letters with a stroke or slash, which have no canonical decomposition.
fn stroke_base(c: char) -> Option<char> {
    Some(match c {
        'đ' => 'd',
        'Đ' => 'D',
        'ł' => 'l',
        'Ł' => 'L',
        'ø' => 'o',
        'Ø' => 'O',
        'ħ' => 'h',
        'Ħ' => 'H',
        'ŧ' => 't',
        'Ŧ' => 'T',
        _ => return None,
    })
}

/// An alef carrying a hamza or madda, or alef wasla.
fn is_alef_variant(c: char) -> bool {
    matches!(c, '\u{0622}' | '\u{0623}' | '\u{0625}' | '\u{0671}' | '\u{0672}' | '\u{0673}')
}

/// Madda, hamza above and hamza below, stored after the letter.
fn is_hamza_mark(c: char) -> bool {
    matches!(c, '\u{0653}'..='\u{0655}')
}

/// Append the folded form of `c` (after the folded character `prev`) to `out`.
fn fold_char(c: char, f: Fold, prev: Option<char>, out: &mut String) {
    if c.is_ascii() {
        out.push(c);
        return;
    }
    if f.kashida && c == TATWEEL {
        return;
    }
    if f.alef_hamza {
        if is_alef_variant(c) {
            out.push(ALEF);
            return;
        }
        if is_hamza_mark(c) && prev == Some(ALEF) {
            return;
        }
    }
    if f.diacritics {
        if is_diacritic(c) {
            return;
        }
        if let Some(b) = stroke_base(c) {
            out.push(b);
            return;
        }
        // The canonical decomposition, used only when it sheds a mark: Hangul syllables and
        // the like stay whole.
        let mut parts = ['\0'; MAX_DECOMP];
        let mut n = 0;
        let mut dropped = false;
        let mut fits = true;
        decompose_canonical(c, |d| {
            if is_diacritic(d) {
                dropped = true;
            } else if let Some(slot) = parts.get_mut(n) {
                *slot = d;
                n += 1;
            } else {
                fits = false;
            }
        });
        if dropped && fits {
            parts.iter().take(n).for_each(|d| out.push(*d));
            return;
        }
    }
    out.push(c);
}

/// A run of original text that folded to something other than itself.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Where its folded form starts in the folded text.
    folded: usize,
    /// Its folded form's length (0 for marks dropped with no letter before them).
    flen: usize,
    /// The original text it stands for.
    start: usize,
    end: usize,
}

/// A folded text with its way back to the original.
#[derive(Debug)]
pub struct Folded {
    pub text: String,
    /// The cuts given to [`fold`], as offsets in the folded text.
    pub cuts: Vec<usize>,
    /// Every unit that is not copied unchanged, in order; text between them is copied byte for byte.
    entries: Vec<Entry>,
}

impl Folded {
    /// The last entry at or before folded offset `x`, or a virtual one at the very start.
    fn entry_at(&self, x: usize) -> Entry {
        let k = self.entries.partition_point(|e| e.folded <= x);
        k.checked_sub(1).and_then(|k| self.entries.get(k)).copied().unwrap_or(Entry { folded: 0, flen: 0, start: 0, end: 0 })
    }

    /// The original offset where the unit holding folded byte `x` starts.
    pub fn start(&self, x: usize) -> usize {
        let e = self.entry_at(x);
        if x < e.folded + e.flen { e.start } else { e.end + (x - e.folded - e.flen) }
    }

    /// The original offset where the unit holding folded byte `x - 1` ends (`x` is a match end).
    pub fn end(&self, x: usize) -> usize {
        let Some(last) = x.checked_sub(1) else { return self.start(0) };
        let e = self.entry_at(last);
        if last < e.folded + e.flen { e.end } else { e.end + (x - e.folded - e.flen) }
    }
}

/// Fold `hay`, whose live text breaks at the sorted byte offsets `cuts` (tracked deletions,
/// paragraph edges): a dropped mark never joins a letter across a cut.
pub fn fold(hay: &str, cuts: &[usize], f: Fold) -> Folded {
    let mut text = String::with_capacity(hay.len());
    let mut fcuts = Vec::with_capacity(cuts.len());
    let mut entries: Vec<Entry> = Vec::new();
    let mut cut = cuts.iter().copied().peekable();
    // The unit last written, while marks may still join it: (entry, is it unchanged so far).
    let mut open: Option<(Entry, bool)> = None;
    let mut prev: Option<char> = None;
    let flush = |open: &mut Option<(Entry, bool)>, entries: &mut Vec<Entry>| {
        if let Some((e, same)) = open.take()
            && !same
        {
            entries.push(e);
        }
    };
    for (i, c) in hay.char_indices() {
        let mut at_cut = false;
        while cut.next_if(|x| *x <= i).is_some() {
            fcuts.push(text.len());
            at_cut = true;
        }
        if at_cut {
            flush(&mut open, &mut entries);
            prev = None;
        }
        let before = text.len();
        fold_char(c, f, prev, &mut text);
        let end = i + c.len_utf8();
        let written = text.get(before..).unwrap_or("");
        if written.is_empty() {
            match open.as_mut() {
                // The mark belongs to the letter before it.
                Some((e, same)) => {
                    e.end = end;
                    *same = false;
                }
                None => entries.push(Entry { folded: before, flen: 0, start: i, end }),
            }
            continue;
        }
        let same = written.len() == c.len_utf8() && written.starts_with(c);
        prev = written.chars().next_back();
        flush(&mut open, &mut entries);
        open = Some((Entry { folded: before, flen: written.len(), start: i, end }, same));
    }
    flush(&mut open, &mut entries);
    fcuts.extend(cut.map(|_| text.len()));
    Folded { text, cuts: fcuts, entries }
}

/// `s` folded, for the Find what text.
pub fn fold_str(s: &str, f: Fold) -> String {
    if !f.any() {
        return s.to_string();
    }
    fold(s, &[], f).text
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Fold = Fold { diacritics: true, kashida: true, alef_hamza: true };

    #[test]
    fn folds_and_maps_back() {
        let hay = "a cafe\u{301} b";
        let f = fold(hay, &[], ALL);
        assert_eq!(f.text, "a cafe b");
        let at = f.text.find("cafe").unwrap();
        // The match ends after the accent, not between the letter and its mark.
        assert_eq!(&hay[f.start(at)..f.end(at + 4)], "cafe\u{301}");
        assert_eq!(&hay[f.start(f.text.find('b').unwrap())..], "b");
        let hay = "ệ x";
        let f = fold(hay, &[], ALL);
        assert_eq!(f.text, "e x");
        assert_eq!((f.start(0), f.end(1), f.start(2)), (0, "ệ".len(), "ệ ".len()));
    }

    #[test]
    fn marks_do_not_join_a_letter_across_a_cut() {
        let hay = "e\u{301}";
        let f = fold(hay, &[1], ALL);
        assert_eq!(f.text, "e");
        assert_eq!(f.cuts, [1]);
        assert_eq!(f.end(1), 1, "the mark after the cut is not part of the `e`");
    }

    #[test]
    fn options_are_separate() {
        let d = Fold { diacritics: true, ..Fold::default() };
        assert_eq!(fold_str("أَحْمَد", d), "أحمد", "hamza stays with only diacritics ignored");
        assert_eq!(fold_str("كـتاب", d), "كـتاب");
        let k = Fold { kashida: true, ..Fold::default() };
        assert_eq!(fold_str("كـتاب", k), "كتاب");
        let a = Fold { alef_hamza: true, ..Fold::default() };
        assert_eq!(fold_str("أ إ آ ٱ \u{627}\u{654}", a), "ا ا ا ا ا");
        assert_eq!(fold_str("ؤ", a), "ؤ", "only alef's hamza");
        assert_eq!(fold_str("한국어", ALL), "한국어", "Hangul is not decomposed");
        assert_eq!(fold_str("đ Ł ø", ALL), "d L o");
    }
}
