//! Thai character name constants for the Thai Unicode block (U+0E00–U+0E5F).
//!
//! Names follow the Unicode Standard Thai block annex.

// ── Consonants — normal (U+0E01–U+0E2E) ──────────────────────────────────────────────────────
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
pub const DOCHADA: char = '\u{0E0E}'; // ฎ  lower-tail consonant
pub const TOPATAK: char = '\u{0E0F}'; // ฏ  lower-tail consonant
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
pub const POPLA: char = '\u{0E1B}'; // ป  upper-tail consonant
pub const PHOPHUNG: char = '\u{0E1C}'; // ผ
pub const FOFA: char = '\u{0E1D}'; // ฝ  upper-tail consonant
pub const PHOPHAN: char = '\u{0E1E}'; // พ
pub const FOFAN: char = '\u{0E1F}'; // ฟ  upper-tail consonant
pub const PHOSAMPHAO: char = '\u{0E20}'; // ภ
pub const MOMA: char = '\u{0E21}'; // ม
pub const YOYAK: char = '\u{0E22}'; // ย
pub const RORUA: char = '\u{0E23}'; // ร
pub const RU: char = '\u{0E24}'; // ฤ  lower-tail consonant
pub const LOLING: char = '\u{0E25}'; // ล
pub const LU: char = '\u{0E26}'; // ฦ  lower-tail consonant
pub const WOWAEN: char = '\u{0E27}'; // ว
pub const SOSALA: char = '\u{0E28}'; // ศ
pub const SORUSI: char = '\u{0E29}'; // ษ
pub const SOSUA: char = '\u{0E2A}'; // ส
pub const HOHIP: char = '\u{0E2B}'; // ห
pub const LOCHULA: char = '\u{0E2C}'; // ฬ
pub const OANG: char = '\u{0E2D}'; // อ
pub const HONOKHUK: char = '\u{0E2E}'; // ฮ

// Punctuation
pub const PAIYANNOI: char = '\u{0E2F}'; // ฯ  ellipsis / abbreviation marker

// ── Vowels and combining marks (U+0E30–U+0E4F) ───────────────────────────────────────────────
pub const SARA_A: char = '\u{0E30}'; // ะ  short /a/, standalone
pub const MAIHUNAKAT: char = '\u{0E31}'; // ั  above vowel (mai han akat)
pub const SARA_AA: char = '\u{0E32}'; // า  long /aː/, standalone (follow vowel)
pub const SARA_AM: char = '\u{0E33}'; // ำ  /am/ composite
pub const SARA_I: char = '\u{0E34}'; // ิ  above vowel
pub const SARA_II: char = '\u{0E35}'; // ี  above vowel
pub const SARA_UE: char = '\u{0E36}'; // ึ  above vowel
pub const SARA_UEE: char = '\u{0E37}'; // ื  above vowel
pub const SARA_U: char = '\u{0E38}'; // ุ  below vowel
pub const SARA_UU: char = '\u{0E39}'; // ู  below vowel
pub const PHINTHU: char = '\u{0E3A}'; // ฺ  Pali mark (phinthu)
pub const BAHT: char = '\u{0E3F}'; // ฿  currency sign

// ── Leading (pre-consonant) vowels (U+0E40–U+0E44) ───────────────────────────────────────────
pub const SARA_E: char = '\u{0E40}'; // เ  leading vowel
pub const SARA_AE: char = '\u{0E41}'; // แ  leading vowel
pub const SARA_O: char = '\u{0E42}'; // โ  leading vowel
pub const MAIMUAN: char = '\u{0E43}'; // ใ  leading vowel
pub const MAIMALAI: char = '\u{0E44}'; // ไ  leading vowel

// ── Other Thai symbols (U+0E45–U+0E4F) ───────────────────────────────────────────────────────
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
pub const NIKHAHIT: char = '\u{0E4D}'; // ํ  Pali mark (nikhahit)
pub const YAMAKKAN: char = '\u{0E4E}'; // ๎  archaic
pub const FONGMAN: char = '\u{0E4F}'; // ๏  section marker

// ── Thai digits (U+0E50–U+0E59) ──────────────────────────────────────────────────────────────
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

// ── Archaic punctuation (U+0E5A–U+0E5B) ──────────────────────────────────────────────────────
pub const ANGKHANKHU: char = '\u{0E5A}'; // ๚  paragraph mark
pub const KHOMUT: char = '\u{0E5B}'; // ๛  end of chapter

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_constants_round_trip() {
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
}
