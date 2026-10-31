//! Thai number formatting for fields and format switches:
//!
//! - `\* BahtText` / `{ BAHTTEXT … }` — number in Thai Baht words (บาทถ้วน / สตางค์)
//! - `\* ThaiNumbers` — traditional Thai numeral glyphs (๐–๙ replacing 0–9)
//! - `\* ThaiCounting` — number spelled out as Thai counting words (สิบสอง for 12)
//!
//! Ported from the gridcraft `BAHTTEXT` function; the algorithm is identical.

/// Thai numeral glyphs U+0E50–U+0E59 (๐๑๒๓๔๕๖๗๘๙), indexed by digit value 0–9.
const THAI_GLYPH_DIGITS: [char; 10] = ['๐', '๑', '๒', '๓', '๔', '๕', '๖', '๗', '๘', '๙'];

/// Replace every ASCII digit in `s` with its Thai numeral glyph; all other characters pass through.
fn digits_to_thai(s: &str) -> String {
    s.chars()
        .map(|c| match c.to_digit(10) {
            Some(d) => THAI_GLYPH_DIGITS.get(d as usize).copied().unwrap_or(c),
            None => c,
        })
        .collect()
}

/// Traditional Thai numeral glyphs for `n`.
/// Each decimal digit of `n` is replaced by its Thai glyph (1 → ๑, 12 → ๑๒, 0 → ๐).
pub(crate) fn thai_digits(n: u32) -> String {
    digits_to_thai(&n.to_string())
}

/// Thai counting words for integer `n` (สิบสอง for 12, ศูนย์ for 0).
/// Used by `\* ThaiCounting` and page/list numbering with `NumFormat::ThaiCounting`.
pub(crate) fn thai_counting_int(n: u32) -> String {
    thai_number(&n.to_string())
}

const THAI_DIGITS: [&str; 10] = ["ศูนย์", "หนึ่ง", "สอง", "สาม", "สี่", "ห้า", "หก", "เจ็ด", "แปด", "เก้า"];
/// Units, tens, hundreds, thousands, ten thousands, hundred thousands.
const THAI_PLACES: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];

/// Thai words for a group of up to six decimal digits (most significant first); empty for zero.
/// One in the tens position is สิบ alone; two is ยี่สิบ; a final one after higher digits is เอ็ด.
/// `after` is true when a higher-order group has already emitted at least one non-zero digit.
fn thai_group(digits: &[u8], after: bool) -> String {
    let n = digits.len();
    let higher = after || digits.iter().take(n.saturating_sub(1)).any(|d| *d != 0);
    let mut out = String::new();
    for (i, &d) in digits.iter().enumerate() {
        let place = n - 1 - i;
        match (place, d) {
            (_, 0) => {}
            (0, 1) if higher => out.push_str("เอ็ด"),
            (1, 1) => out.push_str("สิบ"),
            (1, 2) => out.push_str("ยี่สิบ"),
            _ => {
                out.push_str(THAI_DIGITS.get(d as usize).copied().unwrap_or_default());
                out.push_str(THAI_PLACES.get(place).copied().unwrap_or_default());
            }
        }
    }
    out
}

/// Thai words for a non-negative whole number in decimal digits; zero → "ศูนย์".
fn thai_number(digits: &str) -> String {
    let d: Vec<u8> = digits
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .skip_while(|x| *x == 0)
        .collect();
    if d.is_empty() {
        return "ศูนย์".to_string();
    }
    let first = match d.len() % 6 {
        0 => 6,
        r => r,
    };
    let mut out = String::new();
    let mut start = 0;
    let mut len = first;
    while let Some(group) = d.get(start..start + len) {
        out.push_str(&thai_group(group, start > 0));
        start += len;
        len = 6;
        if start < d.len() {
            out.push_str("ล้าน");
        }
    }
    out
}

/// Round `x` to two decimal places, half away from zero.
fn round_to_satang(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Parse a decimal token (e.g. `"1.005"`) to a satang-rounded `f64` without the
/// precision loss that `s.parse::<f64>().round()` introduces.
///
/// IEEE 754 cannot represent `1.005` exactly; it stores a value slightly below, so
/// `(1.005_f64 * 100.0).round()` = 100, giving `1.00` instead of `1.01`.  This
/// function inspects the decimal string directly: the third fractional digit (if
/// present) decides the rounding carry, and the carry is applied to exact integer
/// arithmetic before the final conversion to `f64`.
///
/// Falls back to `s.parse::<f64>()` for non-decimal tokens (integers, negative
/// numbers, scientific notation).
pub(crate) fn parse_baht_token(s: &str) -> Option<f64> {
    let s = s.trim();
    let negative = s.starts_with('-');
    let abs = if negative { &s[1..] } else { s };

    let (whole_str, frac_str) = match abs.split_once('.') {
        Some(pair) => pair,
        None => return s.parse().ok(), // integer or scientific — no decimal point
    };

    // Only use the decimal path when both halves are purely ASCII digits and the
    // whole part is non-empty.  Fall back to f64 for scientific notation ("1.5e3")
    // or malformed input ("1.2.3") so those values are not silently mis-rounded.
    if whole_str.is_empty()
        || !whole_str.bytes().all(|b| b.is_ascii_digit())
        || !frac_str.bytes().all(|b| b.is_ascii_digit())
    {
        return s.parse().ok();
    }

    // Fractional digits are now guaranteed ASCII digits; no filter needed.
    let digit_at = |i: usize| -> u32 {
        frac_str
            .as_bytes()
            .get(i)
            .copied()
            .map(|b| u32::from(b - b'0'))
            .unwrap_or(0)
    };

    let d0 = digit_at(0);
    let d1 = digit_at(1);
    let d2 = digit_at(2); // rounding digit

    let satang_raw = d0 * 10 + d1 + u32::from(d2 >= 5);
    let (satang, carry) = if satang_raw >= 100 { (satang_raw - 100, 1u64) } else { (satang_raw, 0u64) };

    // Use checked_add: overflow cannot occur with real amounts, but protect
    // against adversarial input (e.g. "18446744073709551615.995").
    let baht: u64 = whole_str.parse::<u64>().ok()?.checked_add(carry)?;
    let magnitude = baht as f64 + satang as f64 / 100.0;
    Some(if negative && magnitude > 0.0 { -magnitude } else { magnitude })
}

/// Thai Baht text for an integer `n` (used by `NumFormat::BahtText.format(n)`).
/// Always ends in "บาท" + "ถ้วน" because page/list numbers are whole.
pub(crate) fn baht_text_int(n: u32) -> String {
    let mut out = thai_number(&n.to_string());
    out.push_str("บาท");
    out.push_str("ถ้วน");
    out
}

/// Thai Baht text for a floating-point amount (used by the `{ BAHTTEXT … }` field).
/// Returns `None` when `x` is not finite.
pub(crate) fn baht_text_f64(x: f64) -> Option<String> {
    let x = round_to_satang(x);
    if !x.is_finite() {
        return None;
    }
    let fixed = format!("{:.2}", x.abs());
    let (whole, frac) = fixed.split_once('.').unwrap_or((fixed.as_str(), "00"));
    let has_baht = whole.bytes().any(|b| b != b'0');
    let has_satang = frac.bytes().any(|b| b != b'0');
    let mut out = String::new();
    if x < 0.0 && (has_baht || has_satang) {
        out.push_str("ลบ");
    }
    if has_baht || !has_satang {
        out.push_str(&thai_number(whole));
        out.push_str("บาท");
    }
    if has_satang {
        out.push_str(&thai_number(frac));
        out.push_str("สตางค์");
    } else {
        out.push_str("ถ้วน");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thai_digits_cases() {
        assert_eq!(thai_digits(0), "๐");
        assert_eq!(thai_digits(1), "๑");
        assert_eq!(thai_digits(9), "๙");
        assert_eq!(thai_digits(12), "๑๒");
        assert_eq!(thai_digits(100), "๑๐๐");
        assert_eq!(thai_digits(1234), "๑๒๓๔");
        assert_eq!(thai_digits(u32::MAX), digits_to_thai(&u32::MAX.to_string()));
    }

    #[test]
    fn thai_counting_int_cases() {
        assert_eq!(thai_counting_int(0),    "ศูนย์");
        assert_eq!(thai_counting_int(1),    "หนึ่ง");
        assert_eq!(thai_counting_int(10),   "สิบ");
        assert_eq!(thai_counting_int(11),   "สิบเอ็ด");
        assert_eq!(thai_counting_int(12),   "สิบสอง");
        assert_eq!(thai_counting_int(20),   "ยี่สิบ");
        assert_eq!(thai_counting_int(21),   "ยี่สิบเอ็ด");
        assert_eq!(thai_counting_int(100),  "หนึ่งร้อย");
        assert_eq!(thai_counting_int(101),  "หนึ่งร้อยเอ็ด");
        assert_eq!(thai_counting_int(1000), "หนึ่งพัน");
        assert_eq!(thai_counting_int(1234), "หนึ่งพันสองร้อยสามสิบสี่");
        assert_eq!(thai_counting_int(1_000_000), "หนึ่งล้าน");
    }

    #[test]
    fn baht_text_f64_cases() {
        let bt = |x: f64| baht_text_f64(x).unwrap_or_default();
        // Microsoft's own example.
        assert_eq!(bt(1234.0), "หนึ่งพันสองร้อยสามสิบสี่บาทถ้วน");
        assert_eq!(bt(0.0),    "ศูนย์บาทถ้วน");
        assert_eq!(bt(21.25),  "ยี่สิบเอ็ดบาทยี่สิบห้าสตางค์");
        assert_eq!(bt(0.5),    "ห้าสิบสตางค์");
        assert_eq!(bt(101.0),  "หนึ่งร้อยเอ็ดบาทถ้วน");
        assert_eq!(bt(1_000_000.0), "หนึ่งล้านบาทถ้วน");
        assert_eq!(bt(11_000_000.0), "สิบเอ็ดล้านบาทถ้วน");
        assert_eq!(bt(-15.5),  "ลบสิบห้าบาทห้าสิบสตางค์");
        assert_eq!(bt(1.999),  "สองบาทถ้วน");
        assert_eq!(bt(0.005),  "หนึ่งสตางค์");
        // A final 1 is เอ็ด after higher digits in the same group or a previous group.
        assert_eq!(bt(21_000_001.0), "ยี่สิบเอ็ดล้านเอ็ดบาทถ้วน");
        assert_eq!(bt(1_000_001.0),  "หนึ่งล้านเอ็ดบาทถ้วน");
        assert_eq!(bt(1001.0),       "หนึ่งพันเอ็ดบาทถ้วน");
        assert_eq!(bt(11.0),         "สิบเอ็ดบาทถ้วน");
        // Non-finite → None.
        assert!(baht_text_f64(f64::INFINITY).is_none());
        assert!(baht_text_f64(f64::NAN).is_none());
    }

    #[test]
    fn baht_text_int_cases() {
        assert_eq!(baht_text_int(1),    "หนึ่งบาทถ้วน");
        assert_eq!(baht_text_int(0),    "ศูนย์บาทถ้วน");
        assert_eq!(baht_text_int(1234), "หนึ่งพันสองร้อยสามสิบสี่บาทถ้วน");
    }

    #[test]
    fn parse_baht_token_rounds_at_string_boundary() {
        let bt = |s: &str| {
            parse_baht_token(s)
                .and_then(baht_text_f64)
                .unwrap_or_default()
        };
        // f64 precision pitfalls: "1.005" stores slightly below 1.005 in IEEE 754
        // so (1.005_f64 * 100.0).round() = 100, giving 1.00 without the string path.
        assert_eq!(bt("1.005"), "หนึ่งบาทหนึ่งสตางค์");  // 1.01 (1 satang = หนึ่ง; เอ็ด only after a higher digit in the same group)
        assert_eq!(bt("0.285"), "ยี่สิบเก้าสตางค์");     // 0.29
        // Exact values still work.
        assert_eq!(bt("1234"), "หนึ่งพันสองร้อยสามสิบสี่บาทถ้วน");
        assert_eq!(bt("0.50"), "ห้าสิบสตางค์");
        assert_eq!(bt("-15.5"), "ลบสิบห้าบาทห้าสิบสตางค์");
        // Carry from satang into baht.
        assert_eq!(bt("1.999"), "สองบาทถ้วน");
        // Non-parseable → None.
        assert!(parse_baht_token("abc").is_none());
        // Scientific notation falls back to f64 (1500.0), not mis-rounded via string path.
        let v = parse_baht_token("1.5e3").unwrap();
        assert!((v - 1500.0).abs() < 1e-9);
        // Malformed decimal ("1.2.3") falls back to f64 parse, which fails → None.
        assert!(parse_baht_token("1.2.3").is_none());
    }
}
