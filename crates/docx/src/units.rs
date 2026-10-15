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
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let pt = match unit.trim() {
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

/// Returns true if string `s` represents an off/false value in ECMA-376 ST_OnOff
/// (0, false, off, f) case-insensitively and ignoring whitespace.
pub fn is_off(s: &str) -> bool {
    let t = s.trim();
    t == "0" || t.eq_ignore_ascii_case("false") || t.eq_ignore_ascii_case("off") || t.eq_ignore_ascii_case("f")
}

/// ST_OnOff: absent `w:val` = on; `0`, `false`, `off` = off.
pub fn on_off(e: &El) -> bool {
    match e.attr("w:val") {
        None => true,
        Some(v) => !is_off(v),
    }
}

/// An attribute parsed as ST_OnOff: present and not off.
pub fn on_off_attr(e: &El, name: &str) -> bool {
    e.attr(name).is_some_and(|v| !is_off(v))
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
        assert_eq!(int("12.6"), Some(13));
        assert_eq!(u32_of("-5"), Some(0));
    }

    #[test]
    fn test_is_off() {
        assert!(is_off("0"));
        assert!(is_off("false"));
        assert!(is_off("False"));
        assert!(is_off("FALSE"));
        assert!(is_off("off"));
        assert!(is_off("Off"));
        assert!(is_off("OFF"));
        assert!(is_off("f"));
        assert!(is_off("F"));
        assert!(is_off(" 0 "));
        assert!(is_off(" off "));
        assert!(!is_off("1"));
        assert!(!is_off("true"));
        assert!(!is_off("True"));
        assert!(!is_off("on"));
        assert!(!is_off("On"));
    }
}
