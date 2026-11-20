//! Unit conversions. HWPUNIT is 1/7200 inch; DOCX uses twips (1/1440 inch, HWPUNIT / 5),
//! half-points for font sizes (height / 50) and EMU for drawings (HWPUNIT × 127).
//!
//! The arithmetic follows the reference implementation exactly (its outputs are compared byte
//! for byte): a fractional HWPUNIT is first cut to a whole number, and rounding sends halves
//! to the even neighbour.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

/// Round half to even (2.5 -> 2, 3.5 -> 4).
pub fn round_even(x: f64) -> i64 {
    if !x.is_finite() {
        return 0;
    }
    let r = x.round(); // half away from zero
    let r = if (x - x.trunc()).abs() == 0.5 && r.rem_euclid(2.0) != 0.0 { r - x.signum() } else { r };
    clamp_i64(r)
}

/// Cut toward zero.
pub fn trunc(x: f64) -> i64 {
    if !x.is_finite() {
        return 0;
    }
    clamp_i64(x.trunc())
}

fn clamp_i64(x: f64) -> i64 {
    // i64 bounds as f64 are inexact at the top; saturate instead of overflowing.
    if x >= 9.2e18 {
        i64::MAX
    } else if x <= -9.2e18 {
        i64::MIN
    } else {
        x as i64
    }
}

/// HWPUNIT -> twips.
pub fn tw(hu: i64) -> i64 {
    round_even(hu as f64 / 5.0)
}

/// A fractional HWPUNIT -> twips (cut to whole HWPUNIT first).
pub fn tw_f(hu: f64) -> i64 {
    tw(trunc(hu))
}

/// HWPUNIT -> EMU, never negative (sizes and page offsets).
pub fn emu(hu: i64) -> i64 {
    hu.saturating_mul(127).max(0)
}

/// HWPUNIT -> EMU keeping the sign (offsets inside drawings).
pub fn emu_signed(hu: f64) -> i64 {
    trunc(hu).saturating_mul(127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_the_reference() {
        assert_eq!(round_even(2.5), 2);
        assert_eq!(round_even(3.5), 4);
        assert_eq!(round_even(-2.5), -2);
        assert_eq!(round_even(2.6), 3);
        assert_eq!(tw(2), 0); // 0.4
        assert_eq!(tw(-7), -1); // -1.4
        assert_eq!(tw(7), 1);
        assert_eq!(tw(5), 1);
        assert_eq!(tw(25), 5);
        assert_eq!(tw(12), 2); // 2.4
        assert_eq!(tw(13), 3); // 2.6
        assert_eq!(tw_f(12.9), 2); // cut to 12 first
        assert_eq!(emu(-3), 0);
        assert_eq!(emu_signed(-3.7), -381);
    }
}
