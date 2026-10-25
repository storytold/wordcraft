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
}
