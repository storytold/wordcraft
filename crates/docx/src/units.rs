//! Lenient number parsing for OOXML attribute values.

use crate::xml::El;

/// Largest absolute length we accept, points (beyond any real page: 200 inches).
pub const MAX_LEN_PT: f32 = 14_400.0;

/// Parse a number, accepting decimals and ISO 29500 universal measures (`1in`, `2.5cm`, `12pt`,
/// `3mm`, `1pc`, `1pi`). `per_pt` is how many file units make a point when there's no unit
/// suffix (20 for twips, 2 for half-points, 8 for eighths, 12700 for EMU). Returns points.
pub fn measure(s: &str, per_pt: f32) -> Option<f32> {
    let t = s.trim();
    if t.is_empty() || t.len() > 40 {
        return None;
    }
    let bytes = t.as_bytes();
    let mut split = t.len();
    let mut in_exp = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() || b == b'.' {
            in_exp = false;
            continue;
        }
        if (b == b'-' || b == b'+') && (i == 0 || in_exp) {
            in_exp = false;
            continue;
        }
        if b == b'e' || b == b'E' {
            let rest = &bytes[i + 1..];
            let after_sign = match rest.first() {
                Some(&s) if s == b'+' || s == b'-' => &rest[1..],
                _ => rest,
            };
            if after_sign.first().is_some_and(|c| c.is_ascii_digit()) {
                in_exp = true;
                continue;
            }
        }
        split = i;
        break;
    }
    let (num, unit) = t.split_at(split);
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let unit_lower = unit.trim().to_ascii_lowercase();
    let pt = match unit_lower.as_str() {
        "" => v / per_pt as f64,
        "pt" => v,
        "in" => v * 72.0,
        "cm" => v * 72.0 / 2.54,
        "mm" => v * 72.0 / 25.4,
        "pc" | "pi" => v * 12.0,
        "px" => v * 0.75,
        "emu" => v / 12_700.0,
        _ => return None,
    };
    let pt = pt as f32;
    pt.is_finite().then_some(pt.clamp(-MAX_LEN_PT, MAX_LEN_PT))
}

/// Plain integer (accepts decimals, rounds; clamps into i64 range).
pub fn int(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() || t.len() > 40 {
        return None;
    }
    if let Ok(v) = t.parse::<i64>() {
        return Some(v);
    }
    let v: f64 = t.parse().ok()?;
    v.is_finite().then(|| v.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64)
}

pub fn u32_of(s: &str) -> Option<u32> {
    int(s).map(|v| v.clamp(0, u32::MAX as i64) as u32)
}

/// Twips attribute → points.
pub fn tw(e: &El, attr: &str) -> Option<f32> {
    e.attr(attr).and_then(|v| measure(v, 20.0))
}

/// ST_OnOff: absent `w:val` = on; `0`, `false`, `off` = off.
pub fn on_off(e: &El) -> bool {
    match e.attr("w:val") {
        None => true,
        Some(v) => !matches!(v.trim(), "0" | "false" | "off" | "False" | "FALSE"),
    }
}

/// An on/off child of `parent`, if present.
pub fn flag(parent: &El, name: &str) -> Option<bool> {
    parent.child(name).map(on_off)
}

/// Format a number for an attribute: integers without a fraction.
pub fn n(v: i64) -> String {
    v.to_string()
}

/// Points → twips string.
pub fn twips(pt: f32) -> String {
    wordcraft_geom::to_twips(pt.clamp(-MAX_LEN_PT, MAX_LEN_PT)).to_string()
}

/// Points → EMU string (never negative for sizes).
pub fn emu(pt: f32) -> String {
    wordcraft_geom::to_emu(pt.clamp(-MAX_LEN_PT, MAX_LEN_PT)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures() {
        assert_eq!(measure("1440", 20.0), Some(72.0));
        assert_eq!(measure("1in", 20.0), Some(72.0));
        assert_eq!(measure("12pt", 2.0), Some(12.0));
        assert_eq!(measure("24.0", 2.0), Some(12.0));
        assert_eq!(measure("nan", 20.0), None);
        assert_eq!(measure("1e400", 20.0), None);
        assert_eq!(measure("99999999999999", 20.0), Some(MAX_LEN_PT));
        assert_eq!(measure("x", 20.0), None);
        assert_eq!(measure("12700emu", 20.0), Some(1.0));
        assert_eq!(measure("25400EMU", 20.0), Some(2.0));
        assert_eq!(measure("1IN", 20.0), Some(72.0));
        assert_eq!(measure("12PT", 2.0), Some(12.0));
        assert_eq!(measure("2.54CM", 20.0), Some(72.0));
        assert_eq!(measure("25.4MM", 20.0), Some(72.0));
        assert_eq!(measure("10PX", 20.0), Some(7.5));
        assert_eq!(measure("1e2pt", 20.0), Some(100.0));
        assert_eq!(measure("1e+2pt", 20.0), Some(100.0));
        assert_eq!(int("12.6"), Some(13));
        assert_eq!(u32_of("-5"), Some(0));
    }
}
