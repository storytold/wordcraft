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
    const UNITS: &[&str] = &["emu", "pt", "in", "cm", "mm", "pc", "pi", "px"];
    let (num, unit) = if let Some(u) = UNITS.iter().find(|&&u| t.to_ascii_lowercase().ends_with(u)) {
        let (n, _) = t.split_at(t.len() - u.len());
        (n.trim(), *u)
    } else {
        (t, "")
    };
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let pt = match unit {
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
        Some(v) => {
            let s = v.trim();
            s != "0" && !s.eq_ignore_ascii_case("false") && !s.eq_ignore_ascii_case("off")
        }
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
        assert_eq!(measure("12700emu", 20.0), Some(1.0));
        assert_eq!(measure("12700EMU", 20.0), Some(1.0));
        assert_eq!(measure("12PT", 2.0), Some(12.0));
        assert_eq!(measure("1IN", 20.0), Some(72.0));
        assert_eq!(int("12.6"), Some(13));
        assert_eq!(u32_of("-5"), Some(0));
    }

    #[test]
    fn test_on_off_casing() {
        let el_on = El { attrs: vec![("w:val".into(), "on".into())], ..Default::default() };
        let el_off_lower = El { attrs: vec![("w:val".into(), "off".into())], ..Default::default() };
        let el_off_mixed = El { attrs: vec![("w:val".into(), "Off".into())], ..Default::default() };
        let el_off_upper = El { attrs: vec![("w:val".into(), "OFF".into())], ..Default::default() };
        let el_false_upper = El { attrs: vec![("w:val".into(), "FALSE".into())], ..Default::default() };
        let el_zero = El { attrs: vec![("w:val".into(), "0".into())], ..Default::default() };
        let el_none = El::default();

        assert!(on_off(&el_on));
        assert!(on_off(&el_none));
        assert!(!on_off(&el_off_lower));
        assert!(!on_off(&el_off_mixed));
        assert!(!on_off(&el_off_upper));
        assert!(!on_off(&el_false_upper));
        assert!(!on_off(&el_zero));
    }
}
