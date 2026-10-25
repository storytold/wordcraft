//! Thai number-to-words (บาท/สตางค์) for the BAHTTEXT field and `\* BahtText` format switch.
//!
//! Ported from the gridcraft `BAHTTEXT` function; the algorithm is identical.

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

    let digit_at = |i: usize| -> u32 {
        frac_str
            .as_bytes()
            .get(i)
            .copied()
            .filter(|b| b.is_ascii_digit())
            .map(|b| u32::from(b - b'0'))
            .unwrap_or(0)
    };

    let d0 = digit_at(0);
    let d1 = digit_at(1);
    let d2 = digit_at(2); // rounding digit

    let satang_raw = d0 * 10 + d1 + u32::from(d2 >= 5);
    let (satang, carry) = if satang_raw >= 100 { (satang_raw - 100, 1u64) } else { (satang_raw, 0u64) };

    let baht: u64 = whole_str.parse::<u64>().ok()? + carry;
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
    }
}
