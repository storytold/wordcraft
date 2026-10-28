//! Thai character name constants, type-class flags, and the per-character type table.
//!
//! Each of the 96 codepoints in the Thai Unicode block (U+0E00–U+0E5F) is assigned a
//! bitmask of [`TCT_*`] flags that describe how it participates in layout, rendering,
//! and input-sequence checking.  The classification is derived from the Unicode Standard
//! (Thai block, Chapter 16) and TIS-620 (Thai Industrial Standard 620-2533), which define
//! the script properties of every Thai character.  Call [`thai_char_type`] to obtain the
//! flags for any codepoint.
//!
//! ## Character type flags
//!
//! ```text
//! Bit  Symbol              Meaning
//! ───  ──────────────────  ─────────────────────────────────────────────
//!  0   TCT_CONSONANCE      Consonant (TCB base character)
//!  1   TCT_DIGIT           Digit
//!  2   TCT_PUNCT           Punctuation
//!  3   TCT_CAPITAL         Capital letter
//!  4   TCT_THAI            Thai character
//!  5   TCT_ENGLISH         English/Latin character
//!  6   TCT_MATH            Mathematical sign
//!  7   TCT_UPPER           Above-baseline glyph (upper tail / above vowel)
//!  8   TCT_LOWER           Below-baseline glyph (lower tail / below vowel)
//!  9   TCT_VOWEL           Vowel (includes MAIHUNAKAT)
//! 10   TCT_TONEMARK        Tone mark (KARAN/THANTHAKHAT included)
//! 11   TCT_LEFTALIGNED     Left-aligned combining mark
//! 12   TCT_BREAK           Word/line-break opportunity
//! 13   TCT_DIACRITIC       Diacritic (MAITAIKHU)
//! 14   TCT_LEADINGVOWEL    Leading vowel (เ แ โ ใ ไ)
//! ```
//!
//! ## Notes on specific characters
//! Some characters have rendering vs. input-sequence interpretations that differ:
//!
//! | Character | Rendering flag | WordCraft input logic |
//! |-----------|----------------|-----------------------|
//! | PHINTHU ◌ฺ (U+0E3A) | `TCT_THAILOWERVOWEL` | Pali mark — not a vowel |
//! | NIKHAHIT ◌ํ (U+0E4D) | `TCT_THAI` (plain) | Pali mark |
//! | SARA_AM ำ (U+0E33) | `TCT_THAIVOWEL` | Upper vowel (occupies vowel slot) |
//!
//! [`thai_char_type`] returns the rendering-oriented flags.
//! The input-sequence checker applies its own classification on top of these.

// ── Character name constants ───────────────────────────────────────────────────────────────
//
// Names follow the Unicode Standard Thai block.  Thai glyphs are shown in comments for quick visual reference.

// Consonants — normal (U+0E01–U+0E2E)
pub const KOKAI: char = '\u{0E01}'; // ก
pub const KHOKHAI: char = '\u{0E02}'; // ข
pub const KHOKHUAT: char = '\u{0E03}'; // ฃ
pub const KHOKHWAI: char = '\u{0E04}'; // ค
pub const KHOKHON: char = '\u{0E05}'; // ฅ
pub const KHORAKHANG: char = '\u{0E06}'; // ฆ
pub const NGONGU: char = '\u{0E07}'; // ง
pub const CHOCHAN: char = '\u{0E08}'; // จ
pub const CHOCHING: char = '\u{0E09}'; // ฉ
pub const CHOCHANG: char = '\u{0E0A}'; // ช
pub const SOSO: char = '\u{0E0B}'; // ซ
pub const CHOCHOE: char = '\u{0E0C}'; // ฌ
pub const YOYING: char = '\u{0E0D}'; // ญ  (short-tail form: U+F70F in WTT 2.0)
pub const DOCHADA: char = '\u{0E0E}'; // ฎ  TCT_THAICONSLOWER (lower tail)
pub const TOPATAK: char = '\u{0E0F}'; // ฏ  TCT_THAICONSLOWER (lower tail)
pub const THOTHAN: char = '\u{0E10}'; // ฐ  (short-tail form: U+F700 in WTT 2.0)
pub const THONANGMONTHO: char = '\u{0E11}'; // ฑ
pub const THOPHUTHAO: char = '\u{0E12}'; // ฒ
pub const NONEN: char = '\u{0E13}'; // ณ
pub const DODEK: char = '\u{0E14}'; // ด
pub const TOTAO: char = '\u{0E15}'; // ต
pub const THOTHUNG: char = '\u{0E16}'; // ถ
pub const THOTHAHAN: char = '\u{0E17}'; // ท
pub const THOTHONG: char = '\u{0E18}'; // ธ
pub const NONU: char = '\u{0E19}'; // น
pub const BOBAIMAI: char = '\u{0E1A}'; // บ
pub const POPLA: char = '\u{0E1B}'; // ป  TCT_THAICONSUPPER (upper tail)
pub const PHOPHUNG: char = '\u{0E1C}'; // ผ
pub const FOFA: char = '\u{0E1D}'; // ฝ  TCT_THAICONSUPPER (upper tail)
pub const PHOPHAN: char = '\u{0E1E}'; // พ
pub const FOFAN: char = '\u{0E1F}'; // ฟ  TCT_THAICONSUPPER (upper tail)
pub const PHOSAMPHAO: char = '\u{0E20}'; // ภ
pub const MOMA: char = '\u{0E21}'; // ม
pub const YOYAK: char = '\u{0E22}'; // ย
pub const RORUA: char = '\u{0E23}'; // ร
pub const RU: char = '\u{0E24}'; // ฤ  TCT_THAICONSLOWER (lower tail)
pub const LOLING: char = '\u{0E25}'; // ล
pub const LU: char = '\u{0E26}'; // ฦ  TCT_THAICONSLOWER (lower tail)
pub const WOWAEN: char = '\u{0E27}'; // ว
pub const SOSALA: char = '\u{0E28}'; // ศ
pub const SORUSI: char = '\u{0E29}'; // ษ
pub const SOSUA: char = '\u{0E2A}'; // ส
pub const HOHIP: char = '\u{0E2B}'; // ห
pub const LOCHULA: char = '\u{0E2C}'; // ฬ
pub const OANG: char = '\u{0E2D}'; // อ
pub const HONOKHUK: char = '\u{0E2E}'; // ฮ

// Punctuation
pub const PAIYANNOI: char = '\u{0E2F}'; // ฯ  TCT_THAIBREAK

// Vowels and combining marks (U+0E30–U+0E4F)
pub const SARA_A: char = '\u{0E30}'; // ะ  short /a/, standalone
pub const MAIHUNAKAT: char = '\u{0E31}'; // ั  above vowel (mai han akat)
pub const SARA_AA: char = '\u{0E32}'; // า  long /aː/, standalone (follow vowel)
pub const SARA_AM: char = '\u{0E33}'; // ำ  /am/ composite; see module doc
pub const SARA_I: char = '\u{0E34}'; // ิ  above vowel
pub const SARA_II: char = '\u{0E35}'; // ี  above vowel
pub const SARA_UE: char = '\u{0E36}'; // ึ  above vowel
pub const SARA_UEE: char = '\u{0E37}'; // ื  above vowel
pub const SARA_U: char = '\u{0E38}'; // ุ  below vowel
pub const SARA_UU: char = '\u{0E39}'; // ู  below vowel
pub const PHINTHU: char = '\u{0E3A}'; // ฺ  Pali mark (phinthu); see module doc
pub const BAHT: char = '\u{0E3F}'; // ฿  currency sign

// Leading (pre-consonant) vowels (U+0E40–U+0E44)
pub const SARA_E: char = '\u{0E40}'; // เ  leading vowel
pub const SARA_AE: char = '\u{0E41}'; // แ  leading vowel
pub const SARA_O: char = '\u{0E42}'; // โ  leading vowel
pub const MAIMUAN: char = '\u{0E43}'; // ใ  leading vowel
pub const MAIMALAI: char = '\u{0E44}'; // ไ  leading vowel

// Other Thai symbols (U+0E45–U+0E4F)
pub const LAKKHANGYAO: char = '\u{0E45}'; // ๅ
pub const MAIYAMOK: char = '\u{0E46}'; // ๆ  repetition mark
pub const MAITAIKHU: char = '\u{0E47}'; // ็  diacritic (shortening mark)
pub const MAIEK: char = '\u{0E48}'; // ่  tone mark (falling)
pub const MAITHO: char = '\u{0E49}'; // ้  tone mark (low)
pub const MAITRI: char = '\u{0E4A}'; // ๊  tone mark (high)
pub const MAICHATTAWA: char = '\u{0E4B}'; // ๋  tone mark (rising)
pub const THANTHAKHAT: char = '\u{0E4C}'; // ์  silent-consonant marker (karan)
/// Alias for [`THANTHAKHAT`] — both names appear in the original source.
pub const KARAN: char = THANTHAKHAT;
pub const NIKHAHIT: char = '\u{0E4D}'; // ํ  Pali mark (nikhahit); see module doc
pub const YAMAKKAN: char = '\u{0E4E}'; // ๎  archaic
pub const FONGMAN: char = '\u{0E4F}'; // ๏  section marker

// Thai digits (U+0E50–U+0E59)
pub const THAIZERO: char = '\u{0E50}'; // ๐
pub const THAIONE: char = '\u{0E51}'; // ๑
pub const THAITWO: char = '\u{0E52}'; // ๒
pub const THAITHREE: char = '\u{0E53}'; // ๓
pub const THAIFOUR: char = '\u{0E54}'; // ๔
pub const THAIFIVE: char = '\u{0E55}'; // ๕
pub const THAISIX: char = '\u{0E56}'; // ๖
pub const THAISEVEN: char = '\u{0E57}'; // ๗
pub const THAIEIGHT: char = '\u{0E58}'; // ๘
pub const THAININE: char = '\u{0E59}'; // ๙

// Archaic punctuation (U+0E5A–U+0E5B)
pub const ANGKHANKHU: char = '\u{0E5A}'; // ๚  paragraph mark
pub const KHOMUT: char = '\u{0E5B}'; // ๛  end of chapter

// ── Type-class flags (TCT_*) ───────────────────────────────────────────────────────────────
//
// Bit layout:
//   fedcba9876543210
//   ||||||||||||||||
//   ||||||||||||||||+── TCT_CONSONANCE   0x0001  consonant
//   |||||||||||||||+─── TCT_DIGIT        0x0002  digit
//   ||||||||||||||+──── TCT_PUNCT        0x0004  punctuation
//   |||||||||||||+───── TCT_CAPITAL      0x0008  capital
//   ||||||||||||+────── TCT_THAI         0x0010  Thai
//   |||||||||||+─────── TCT_ENGLISH      0x0020  English/Latin
//   ||||||||||+──────── TCT_MATH         0x0040  math
//   |||||||||+───────── TCT_UPPER        0x0080  above-baseline (upper tail / above vowel)
//   ||||||||+────────── TCT_LOWER        0x0100  below-baseline (lower tail / below vowel)
//   |||||||+─────────── TCT_VOWEL        0x0200  vowel
//   ||||||+──────────── TCT_TONEMARK     0x0400  tone mark (incl. karan)
//   |||||+───────────── TCT_LEFTALIGNED  0x0800  left-aligned combining
//   ||||+────────────── TCT_BREAK        0x1000  break opportunity
//   |||+─────────────── TCT_DIACRITIC    0x2000  diacritic
//   ||+──────────────── TCT_LEADINGVOWEL 0x4000  leading (pre-consonant) vowel
//   |+───────────────── (reserved)       0x8000
//   +────────────────── (reserved)

/// No flags — control character or unassigned.
pub const TCT_CONTROL: u16 = 0x0000;
pub const TCT_CONSONANCE: u16 = 0x0001;
pub const TCT_DIGIT: u16 = 0x0002;
pub const TCT_PUNCT: u16 = 0x0004;
pub const TCT_CAPITAL: u16 = 0x0008;
pub const TCT_THAI: u16 = 0x0010;
pub const TCT_ENGLISH: u16 = 0x0020;
pub const TCT_MATH: u16 = 0x0040;
/// Above-baseline: upper-tail consonant or above vowel.
pub const TCT_UPPER: u16 = 0x0080;
/// Below-baseline: lower-tail consonant or below vowel.
pub const TCT_LOWER: u16 = 0x0100;
pub const TCT_VOWEL: u16 = 0x0200;
/// Tone mark.  THANTHAKHAT (karan/silent marker) is included.
pub const TCT_TONEMARK: u16 = 0x0400;
pub const TCT_LEFTALIGNED: u16 = 0x0800;
pub const TCT_BREAK: u16 = 0x1000;
/// Diacritic.  Only MAITAIKHU (◌็ U+0E47) in standard Thai.
pub const TCT_DIACRITIC: u16 = 0x2000;
/// Leading (pre-consonant) vowel: เ แ โ ใ ไ.
pub const TCT_LEADINGVOWEL: u16 = 0x4000;

// ── Composed types ─────────────────────────────────────────────────────────────────────────

/// Mask for all zero-width combining marks (vowels + diacritics + tone marks).
pub const TCT_THAIZEROWIDTH: u16 = TCT_VOWEL | TCT_DIACRITIC | TCT_TONEMARK;

/// Normal Thai consonant.
pub const TCT_THAICONS: u16 = TCT_THAI | TCT_CONSONANCE;
/// Lower-tail consonant (e.g. DOCHADA ฎ, TOPATAK ฏ, RU ฤ, LU ฦ).
pub const TCT_THAICONSLOWER: u16 = TCT_THAICONS | TCT_LOWER;
/// Upper-tail consonant (e.g. POPLA ป, FOFA ฝ, FOFAN ฟ).
pub const TCT_THAICONSUPPER: u16 = TCT_THAICONS | TCT_UPPER;

pub const TCT_THAIVOWEL: u16 = TCT_THAI | TCT_VOWEL;
/// Above (upper) vowel: ั ิ ี ึ ื.
pub const TCT_THAIUPPERVOWEL: u16 = TCT_THAI | TCT_VOWEL | TCT_UPPER;
/// Below (lower) vowel: ุ ู (and ฺ phinthu in the original table).
pub const TCT_THAILOWERVOWEL: u16 = TCT_THAI | TCT_VOWEL | TCT_LOWER;
pub const TCT_THAIUPPERVOWELLEFT: u16 = TCT_THAIUPPERVOWEL | TCT_LEFTALIGNED;
pub const TCT_THAILOWERVOWELLEFT: u16 = TCT_THAILOWERVOWEL | TCT_LEFTALIGNED;

pub const TCT_THAIDIGIT: u16 = TCT_THAI | TCT_DIGIT;
pub const TCT_THAITONE: u16 = TCT_THAI | TCT_TONEMARK;
pub const TCT_THAIDIAC: u16 = TCT_THAI | TCT_DIACRITIC;
pub const TCT_THAIBREAK: u16 = TCT_THAI | TCT_BREAK;
pub const TCT_SEPERATOR: u16 = TCT_BREAK | TCT_PUNCT;
pub const TCT_ENGCONS: u16 = TCT_ENGLISH | TCT_CONSONANCE;
pub const TCT_ENGCAPS: u16 = TCT_ENGLISH | TCT_CAPITAL | TCT_CONSONANCE;
pub const TCT_ENGDIGIT: u16 = TCT_ENGLISH | TCT_DIGIT;

// ── Static type table for the Thai Unicode block (U+0E00–U+0E5F) ──────────────────────────
//
// Derived from the Unicode Standard (Thai block, Chapter 16) and TIS-620.
// Index = codepoint − U+0E00 (so KOKAI U+0E01 is at index 1).
//
// NOTE: PHINTHU (U+0E3A, index 58) is classified as TCT_THAILOWERVOWEL for rendering;
// NIKHAHIT (U+0E4D, index 77) is plain TCT_THAI.  WordCraft's input logic treats both
// as Pali marks.  SARA_AM (U+0E33, index 51) is TCT_THAIVOWEL here; the input logic
// uses it as UpperVowel.
//
// Entries beyond U+0E5B (indices 92–95) are unassigned in Unicode → TCT_CONTROL.

#[allow(clippy::doc_markdown)]
static THAI_CHAR_TYPE: [u16; 96] = [
    // ── U+0E00–0E0F ────────────────────────────────────────────────────────────
    TCT_BREAK,         // U+0E00  (unassigned; TIS-620 0xA0 non-breaking space)
    TCT_THAICONS,      // U+0E01  KOKAI ก
    TCT_THAICONS,      // U+0E02  KHOKHAI ข
    TCT_THAICONS,      // U+0E03  KHOKHUAT ฃ
    TCT_THAICONS,      // U+0E04  KHOKHWAI ค
    TCT_THAICONS,      // U+0E05  KHOKHON ฅ
    TCT_THAICONS,      // U+0E06  KHORAKHANG ฆ
    TCT_THAICONS,      // U+0E07  NGONGU ง
    TCT_THAICONS,      // U+0E08  CHOCHAN จ
    TCT_THAICONS,      // U+0E09  CHOCHING ฉ
    TCT_THAICONS,      // U+0E0A  CHOCHANG ช
    TCT_THAICONS,      // U+0E0B  SOSO ซ
    TCT_THAICONS,      // U+0E0C  CHOCHOE ฌ
    TCT_THAICONS,      // U+0E0D  YOYING ญ
    TCT_THAICONSLOWER, // U+0E0E  DOCHADA ฎ
    TCT_THAICONSLOWER, // U+0E0F  TOPATAK ฏ
    // ── U+0E10–0E1F ────────────────────────────────────────────────────────────
    TCT_THAICONS,      // U+0E10  THOTHAN ฐ
    TCT_THAICONS,      // U+0E11  THONANGMONTHO ฑ
    TCT_THAICONS,      // U+0E12  THOPHUTHAO ฒ
    TCT_THAICONS,      // U+0E13  NONEN ณ
    TCT_THAICONS,      // U+0E14  DODEK ด
    TCT_THAICONS,      // U+0E15  TOTAO ต
    TCT_THAICONS,      // U+0E16  THOTHUNG ถ
    TCT_THAICONS,      // U+0E17  THOTHAHAN ท
    TCT_THAICONS,      // U+0E18  THOTHONG ธ
    TCT_THAICONS,      // U+0E19  NONU น
    TCT_THAICONS,      // U+0E1A  BOBAIMAI บ
    TCT_THAICONSUPPER, // U+0E1B  POPLA ป
    TCT_THAICONS,      // U+0E1C  PHOPHUNG ผ
    TCT_THAICONSUPPER, // U+0E1D  FOFA ฝ
    TCT_THAICONS,      // U+0E1E  PHOPHAN พ
    TCT_THAICONSUPPER, // U+0E1F  FOFAN ฟ
    // ── U+0E20–0E2F ────────────────────────────────────────────────────────────
    TCT_THAICONS,      // U+0E20  PHOSAMPHAO ภ
    TCT_THAICONS,      // U+0E21  MOMA ม
    TCT_THAICONS,      // U+0E22  YOYAK ย
    TCT_THAICONS,      // U+0E23  RORUA ร
    TCT_THAICONSLOWER, // U+0E24  RU ฤ
    TCT_THAICONS,      // U+0E25  LOLING ล
    TCT_THAICONSLOWER, // U+0E26  LU ฦ
    TCT_THAICONS,      // U+0E27  WOWAEN ว
    TCT_THAICONS,      // U+0E28  SOSALA ศ
    TCT_THAICONS,      // U+0E29  SORUSI ษ
    TCT_THAICONS,      // U+0E2A  SOSUA ส
    TCT_THAICONS,      // U+0E2B  HOHIP ห
    TCT_THAICONS,      // U+0E2C  LOCHULA ฬ
    TCT_THAICONS,      // U+0E2D  OANG อ
    TCT_THAICONS,      // U+0E2E  HONOKHUK ฮ
    TCT_THAIBREAK,     // U+0E2F  PAIYANNOI ฯ
    // ── U+0E30–0E3F ────────────────────────────────────────────────────────────
    TCT_THAI,           // U+0E30  SARA_A ะ  (standalone short /a/)
    TCT_THAIUPPERVOWEL, // U+0E31  MAIHUNAKAT ั
    TCT_THAI,           // U+0E32  SARA_AA า  (standalone long /aː/)
    TCT_THAIVOWEL,      // U+0E33  SARA_AM ำ  (see module doc for input-logic note)
    TCT_THAIUPPERVOWEL, // U+0E34  SARA_I ิ
    TCT_THAIUPPERVOWEL, // U+0E35  SARA_II ี
    TCT_THAIUPPERVOWEL, // U+0E36  SARA_UE ึ
    TCT_THAIUPPERVOWEL, // U+0E37  SARA_UEE ื
    TCT_THAILOWERVOWEL, // U+0E38  SARA_U ุ
    TCT_THAILOWERVOWEL, // U+0E39  SARA_UU ู
    TCT_THAILOWERVOWEL, // U+0E3A  PHINTHU ฺ  (see module doc for input-logic note)
    TCT_BREAK,          // U+0E3B  (unassigned)
    TCT_BREAK,          // U+0E3C  (unassigned)
    TCT_CONTROL,        // U+0E3D  (unassigned)
    TCT_CONTROL,        // U+0E3E  (unassigned)
    TCT_THAI,           // U+0E3F  BAHT ฿
    // ── U+0E40–0E4F ────────────────────────────────────────────────────────────
    TCT_THAI | TCT_LEADINGVOWEL, // U+0E40  SARA_E เ
    TCT_THAI | TCT_LEADINGVOWEL, // U+0E41  SARA_AE แ
    TCT_THAI | TCT_LEADINGVOWEL, // U+0E42  SARA_O โ
    TCT_THAI | TCT_LEADINGVOWEL, // U+0E43  MAIMUAN ใ
    TCT_THAI | TCT_LEADINGVOWEL, // U+0E44  MAIMALAI ไ
    TCT_THAI,                    // U+0E45  LAKKHANGYAO ๅ
    TCT_THAI,                    // U+0E46  MAIYAMOK ๆ
    TCT_THAIDIAC,                // U+0E47  MAITAIKHU ็
    TCT_THAITONE,                // U+0E48  MAIEK ่
    TCT_THAITONE,                // U+0E49  MAITHO ้
    TCT_THAITONE,                // U+0E4A  MAITRI ๊
    TCT_THAITONE,                // U+0E4B  MAICHATTAWA ๋
    TCT_THAITONE,                // U+0E4C  THANTHAKHAT ์ (karan)
    TCT_THAI,                    // U+0E4D  NIKHAHIT ํ  (see module doc for input-logic note)
    TCT_THAI,                    // U+0E4E  YAMAKKAN ๎
    TCT_THAI,                    // U+0E4F  FONGMAN ๏
    // ── U+0E50–0E5F ────────────────────────────────────────────────────────────
    TCT_THAIDIGIT, // U+0E50  THAIZERO ๐
    TCT_THAIDIGIT, // U+0E51  THAIONE ๑
    TCT_THAIDIGIT, // U+0E52  THAITWO ๒
    TCT_THAIDIGIT, // U+0E53  THAITHREE ๓
    TCT_THAIDIGIT, // U+0E54  THAIFOUR ๔
    TCT_THAIDIGIT, // U+0E55  THAIFIVE ๕
    TCT_THAIDIGIT, // U+0E56  THAISIX ๖
    TCT_THAIDIGIT, // U+0E57  THAISEVEN ๗
    TCT_THAIDIGIT, // U+0E58  THAIEIGHT ๘
    TCT_THAIDIGIT, // U+0E59  THAININE ๙
    TCT_THAIBREAK, // U+0E5A  ANGKHANKHU ๚
    TCT_THAIBREAK, // U+0E5B  KHOMUT ๛
    TCT_CONTROL,   // U+0E5C  (unassigned)
    TCT_CONTROL,   // U+0E5D  (unassigned)
    TCT_CONTROL,   // U+0E5E  (unassigned)
    TCT_CONTROL,   // U+0E5F  (unassigned)
];

/// Return the `TCT_*` type flags for a character.
///
/// For Thai characters in U+0E00–U+0E5F this performs an O(1) table lookup.
/// All other codepoints return `TCT_CONTROL` (zero).
///
/// The returned flags reflect the *rendering/layout* classification.
/// The input-sequence checker applies its own finer-grained classification on top of these.
#[inline]
pub fn thai_char_type(c: char) -> u16 {
    let cp = u32::from(c);
    if cp >= 0x0E00 && cp <= 0x0E5F { THAI_CHAR_TYPE[(cp - 0x0E00) as usize] } else { TCT_CONTROL }
}

// ── Convenience predicates ─────────────────────────────────────────────────────────────────

/// Returns `true` if `c` is any Thai consonant (normal, lower-tail, or upper-tail).
#[inline]
pub fn is_thai_consonant(c: char) -> bool {
    thai_char_type(c) & TCT_THAICONS == TCT_THAICONS
}

/// Returns `true` if `c` is a Thai vowel (any of the zero-width combining vowels).
#[inline]
pub fn is_thai_vowel(c: char) -> bool {
    let t = thai_char_type(c);
    t & (TCT_THAI | TCT_VOWEL) == (TCT_THAI | TCT_VOWEL)
}

/// Returns `true` if `c` is a Thai above (upper) vowel: ั ิ ี ึ ื.
#[inline]
pub fn is_thai_upper_vowel(c: char) -> bool {
    thai_char_type(c) & TCT_THAIUPPERVOWEL == TCT_THAIUPPERVOWEL
}

/// Returns `true` if `c` is a Thai below (lower) vowel: ุ ู.
///
/// Note: PHINTHU ◌ฺ (U+0E3A) is also flagged `TCT_THAILOWERVOWEL` in the rendering
/// table but is treated as a Pali mark by the input-sequence checker.
#[inline]
pub fn is_thai_lower_vowel(c: char) -> bool {
    thai_char_type(c) & TCT_THAILOWERVOWEL == TCT_THAILOWERVOWEL
}

/// Returns `true` if `c` is a Thai tone mark (่ ้ ๊ ๋ ์).
#[inline]
pub fn is_thai_tone(c: char) -> bool {
    thai_char_type(c) & TCT_THAITONE == TCT_THAITONE
}

/// Returns `true` if `c` is the Thai diacritic MAITAIKHU (◌็ U+0E47).
#[inline]
pub fn is_thai_diacritic(c: char) -> bool {
    thai_char_type(c) & TCT_THAIDIAC == TCT_THAIDIAC
}

/// Returns `true` if `c` is a leading (pre-consonant) vowel: เ แ โ ใ ไ.
#[inline]
pub fn is_thai_leading_vowel(c: char) -> bool {
    thai_char_type(c) & TCT_LEADINGVOWEL != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_constants_round_trip() {
        // Every named constant must map back to its own codepoint.
        assert_eq!(KOKAI as u32, 0x0E01);
        assert_eq!(YOYING as u32, 0x0E0D);
        assert_eq!(THOTHAN as u32, 0x0E10);
        assert_eq!(SARA_AM as u32, 0x0E33);
        assert_eq!(PHINTHU as u32, 0x0E3A);
        assert_eq!(NIKHAHIT as u32, 0x0E4D);
        assert_eq!(MAITAIKHU as u32, 0x0E47);
        assert_eq!(THANTHAKHAT as u32, 0x0E4C);
        assert_eq!(KARAN, THANTHAKHAT);
        assert_eq!(KHOMUT as u32, 0x0E5B);
    }

    #[test]
    fn consonant_types() {
        // Normal consonants.
        assert_eq!(thai_char_type(KOKAI), TCT_THAICONS);
        assert_eq!(thai_char_type(YOYING), TCT_THAICONS);
        assert_eq!(thai_char_type(THOTHAN), TCT_THAICONS);
        // Lower-tail consonants.
        assert_eq!(thai_char_type(DOCHADA), TCT_THAICONSLOWER);
        assert_eq!(thai_char_type(TOPATAK), TCT_THAICONSLOWER);
        assert_eq!(thai_char_type(RU), TCT_THAICONSLOWER);
        assert_eq!(thai_char_type(LU), TCT_THAICONSLOWER);
        // Upper-tail consonants.
        assert_eq!(thai_char_type(POPLA), TCT_THAICONSUPPER);
        assert_eq!(thai_char_type(FOFA), TCT_THAICONSUPPER);
        assert_eq!(thai_char_type(FOFAN), TCT_THAICONSUPPER);
    }

    #[test]
    fn vowel_types() {
        // Above vowels.
        assert_eq!(thai_char_type(MAIHUNAKAT), TCT_THAIUPPERVOWEL);
        assert_eq!(thai_char_type(SARA_I), TCT_THAIUPPERVOWEL);
        assert_eq!(thai_char_type(SARA_II), TCT_THAIUPPERVOWEL);
        assert_eq!(thai_char_type(SARA_UE), TCT_THAIUPPERVOWEL);
        assert_eq!(thai_char_type(SARA_UEE), TCT_THAIUPPERVOWEL);
        // Below vowels.
        assert_eq!(thai_char_type(SARA_U), TCT_THAILOWERVOWEL);
        assert_eq!(thai_char_type(SARA_UU), TCT_THAILOWERVOWEL);
        assert_eq!(thai_char_type(PHINTHU), TCT_THAILOWERVOWEL); // rendering table
        // SARA_AM: composite vowel.
        assert_eq!(thai_char_type(SARA_AM), TCT_THAIVOWEL);
        // Leading vowels.
        assert_eq!(thai_char_type(SARA_E) & TCT_LEADINGVOWEL, TCT_LEADINGVOWEL);
        assert_eq!(thai_char_type(MAIMALAI) & TCT_LEADINGVOWEL, TCT_LEADINGVOWEL);
    }

    #[test]
    fn tone_and_diacritic_types() {
        assert_eq!(thai_char_type(MAIEK), TCT_THAITONE);
        assert_eq!(thai_char_type(MAITHO), TCT_THAITONE);
        assert_eq!(thai_char_type(MAITRI), TCT_THAITONE);
        assert_eq!(thai_char_type(MAICHATTAWA), TCT_THAITONE);
        assert_eq!(thai_char_type(THANTHAKHAT), TCT_THAITONE);
        assert_eq!(thai_char_type(MAITAIKHU), TCT_THAIDIAC);
        // NIKHAHIT is plain TCT_THAI in the rendering table.
        assert_eq!(thai_char_type(NIKHAHIT), TCT_THAI);
    }

    #[test]
    fn digit_and_break_types() {
        assert_eq!(thai_char_type(THAIZERO), TCT_THAIDIGIT);
        assert_eq!(thai_char_type(THAININE), TCT_THAIDIGIT);
        assert_eq!(thai_char_type(PAIYANNOI), TCT_THAIBREAK);
        assert_eq!(thai_char_type(ANGKHANKHU), TCT_THAIBREAK);
        assert_eq!(thai_char_type(KHOMUT), TCT_THAIBREAK);
    }

    #[test]
    fn predicates() {
        assert!(is_thai_consonant(KOKAI));
        assert!(is_thai_consonant(POPLA));
        assert!(!is_thai_consonant(SARA_I));

        assert!(is_thai_upper_vowel(SARA_I));
        assert!(!is_thai_upper_vowel(SARA_U));
        assert!(is_thai_lower_vowel(SARA_U));

        assert!(is_thai_tone(MAIEK));
        assert!(is_thai_diacritic(MAITAIKHU));
        assert!(!is_thai_diacritic(MAIEK));

        assert!(is_thai_leading_vowel(SARA_E));
        assert!(is_thai_leading_vowel(MAIMALAI));
        assert!(!is_thai_leading_vowel(SARA_I));

        // Non-Thai returns zero.
        assert_eq!(thai_char_type('a'), TCT_CONTROL);
        assert_eq!(thai_char_type('ก'), TCT_THAICONS); // same as KOKAI
    }
}
