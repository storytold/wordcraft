use super::*;

/// What a piece of text is shaped as: one face, case, script formatting and direction.
#[derive(Clone, Copy)]
pub(super) struct SegKey {
    pub(super) face: Option<FaceRef>,
    pub(super) small: bool,
    /// Formatted with the complex-script properties (Persian, Arabic, Hebrew…).
    pub(super) complex: bool,
    /// Odd bidi level: shaped right to left.
    pub(super) rtl: bool,
}

impl SegKey {
    pub(super) fn same(&self, o: &SegKey) -> bool {
        self.face.map(|f| f.id()) == o.face.map(|f| f.id()) && self.small == o.small && self.complex == o.complex && self.rtl == o.rtl
    }
}

/// The first character of `c`'s capital (ß → S): the shaper's one-to-one mapping, and the
/// text read back for labels and field results, whose byte ranges can't grow.
pub(super) fn upper_char(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// The character drawn for `c` in a run formatted `rc`: All Caps capitalises every letter,
/// Small Caps the lowercase ones (as the shaper draws them). The PDF text layer uses it so
/// the text read back is the text shown (#1473).
pub(crate) fn caps_char(rc: &ResolvedChar, c: char) -> char {
    if rc.caps || (rc.small_caps && c.is_lowercase()) { upper_char(c) } else { c }
}

/// Appends the text drawn for `c` in a run formatted `rc`: like [`caps_char`], but with a
/// letter's full capital (ß → SS), as body text draws it (#1581).
pub(crate) fn push_caps(rc: &ResolvedChar, c: char, out: &mut String) {
    if rc.caps || (rc.small_caps && c.is_lowercase()) { out.extend(c.to_uppercase()) } else { out.push(c) }
}

/// The OpenType ligature features a `w14:ligatures` value sets, each on or off. Required
/// ligatures (`rlig`, Arabic lam-alef) are never turned off.
pub(crate) fn ligature_features(l: wordcraft_doc::props::Ligatures) -> [wordcraft_fonts::Feature; 4] {
    let f = |tag: &[u8; 4], on: bool| wordcraft_fonts::feature_value(tag, on as u32);
    [f(b"liga", l.standard), f(b"clig", l.contextual), f(b"hlig", l.historical), f(b"dlig", l.discretional)]
}

/// A Khmer letter, sign or digit (U+1780–U+17FF, Khmer Symbols U+19E0–U+19FF).
pub(super) fn is_khmer(c: char) -> bool {
    matches!(u32::from(c), 0x1780..=0x17FF | 0x19E0..=0x19FF)
}

/// Whether `c`'s capital is more than one character (ß → SS), so All Caps draws more
/// characters than the document holds.
pub(super) fn caps_expands(c: char) -> bool {
    c.to_uppercase().nth(1).is_some()
}

/// Combining marks and joiners (bidi classes NSM, BN: harakat, ZWNJ, ZWJ) belong to the
/// character before them: same font, same shaping run, so Arabic joining isn't broken.
pub(super) fn is_mark(c: char) -> bool {
    use unicode_bidi::BidiClass::{BN, NSM};
    // No ASCII character is a mark (its BN characters are controls).
    !c.is_ascii() && matches!(unicode_bidi::bidi_class(c), NSM | BN) && !c.is_control()
}

/// A digit Word's Numeral modes substitute: ASCII or Arabic-Indic (U+0660–U+0669).
pub(super) fn is_numeral_digit(c: char) -> bool {
    c.is_ascii_digit() || ('\u{660}'..='\u{669}').contains(&c)
}

/// Characters skipped when looking for the text around a number: digits, spaces and the
/// Arabic decimal/thousands separators (٫ ٬).
fn is_number_inside(c: char) -> bool {
    is_numeral_digit(c) || ('\u{6f0}'..='\u{6f9}').contains(&c) || c == ' ' || c == '\t' || c == '\u{66b}' || c == '\u{66c}'
}

/// Which digits of run `text` sit in right-to-left surroundings (Word › Numeral › Context), by
/// byte offset. European numbers take an even bidi level, so a digit's own level says little;
/// the nearest neighbour on either side that isn't part of the number (digits, spaces,
/// separators) decides. One pass each way, so long runs of digits and spaces stay linear.
/// Offsets past `levels` count as left to right.
pub(super) fn context_digit_dirs(text: &str, levels: &[u8], base: usize) -> Vec<bool> {
    let odd = |o: usize| levels.get(base.saturating_add(o)).is_some_and(|l| l % 2 == 1);
    let mut out = vec![false; text.len()];
    let mut left_rtl = false;
    for (o, c) in text.char_indices() {
        if !is_number_inside(c) {
            left_rtl = odd(o);
        } else if is_numeral_digit(c)
            && let Some(v) = out.get_mut(o)
        {
            *v = odd(o) || left_rtl;
        }
    }
    let mut right_rtl = false;
    for (o, c) in text.char_indices().rev() {
        if !is_number_inside(c) {
            right_rtl = odd(o);
        } else if right_rtl
            && is_numeral_digit(c)
            && let Some(v) = out.get_mut(o)
        {
            *v = true;
        }
    }
    out
}

/// Map a byte range of numeral-substituted display text back to the original text.
///
/// `map_starts[i]` is the display byte offset of the `i`th char and `orig_of[i]` its original
/// byte offset (`orig_of` ends with the original length as a sentinel). Substitution is
/// 1 char → 1 char, so both ends land on char boundaries; anything else falls back to the
/// end of the original text instead of panicking or slicing mid-character.
pub(crate) fn remap_range(map_starts: &[usize], orig_of: &[usize], orig_len: usize, ms: usize, me: usize) -> (usize, usize) {
    let n = map_starts.len();
    let i = map_starts.partition_point(|&b| b < ms).min(n);
    let j = map_starts.partition_point(|&b| b < me).min(n);
    let s = orig_of.get(i).copied().unwrap_or(orig_len);
    let e = orig_of.get(j).copied().unwrap_or(orig_len);
    (s.min(e), s.max(e))
}

/// Word draws a font that isn't installed in Cambria when the document's font table classes it
/// as roman (measured in Word; Calibri otherwise, which our own substitutes already match).
pub(super) const MISSING_ROMAN_FONT: &str = "Cambria";

/// The family to look `font` up as: [`MISSING_ROMAN_FONT`] for a missing roman font, else `font`.
/// A missing Thai font keeps its name: it is drawn in a Thai face, not in Cambria (#1549).
/// A font with substitutes of its own keeps them: Times New Roman is roman in every font table,
/// and where it isn't installed Liberation Serif has its metrics; Word never meets it missing.
pub(super) fn lookup_family<'f>(doc: &Document, font: &'f str) -> &'f str {
    let db = wordcraft_fonts::FontDb::global();
    let roman = doc.settings.roman_fonts.iter().any(|f| f.eq_ignore_ascii_case(font));
    if roman
        && !wordcraft_fonts::word::is_thai_family(font)
        && !wordcraft_doc::symbol_fonts::is_symbol_font(font)
        && !wordcraft_fonts::word::has_named_substitutes(font)
        && !db.has_family(font)
        && !db.has_family(wordcraft_fonts::word::charset_base_family(font))
    {
        MISSING_ROMAN_FONT
    } else {
        font
    }
}

/// One face for a run in a Thai font that isn't installed (#1549): Word substitutes one font
/// (Cordia New) for the whole run, so the Latin letters, digits and ฿ keep the Thai letters' face
/// and size. `None` unless `font` is a Thai family, it was substituted, and its substitute
/// `primary` lacks some of the run's Thai or Latin characters; then the first face covering all of
/// them, if one does. Other families keep Latin text in their own substitute, as Word does.
pub(crate) fn thai_run_face(font: &str, substituted: bool, primary: FaceRef, text: &str, bold: bool, italic: bool) -> Option<FaceRef> {
    if !substituted || !wordcraft_fonts::word::is_thai_family(font) {
        return None;
    }
    let run: Vec<char> = text.chars().filter(|c| thai_or_latin(*c)).collect();
    if run.iter().all(|c| primary.covers(*c)) {
        return None;
    }
    wordcraft_fonts::FontDb::global().fallback_for_run(&run, primary.id(), bold, italic).map(|f| FaceRef::of(&f))
}

/// Text a Thai substitute draws: Thai letters, digits and signs (฿), and Latin letters, digits
/// and punctuation.
fn thai_or_latin(c: char) -> bool {
    matches!(u32::from(c), 0x0021..=0x024F | 0x0E00..=0x0E7F) && !c.is_whitespace() && !c.is_control() && c != SOFT_HYPHEN
}
