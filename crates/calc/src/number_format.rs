//! Result display after conversion to the requested units.

use craft_units::{Formatted, Precision, Quantity, Scale, UnitError, UnitSystem, format, natural_unit};
use wordcraft_doc::calc::{NumberStyle, ResultFormat};

pub(crate) fn quantity(
    value: Quantity,
    scale: Option<&Scale>,
    system: UnitSystem,
    precision: Precision,
    style: Option<ResultFormat>,
) -> Result<Formatted, UnitError> {
    let Some(style) = style else { return format(value, scale, system, precision) };
    let converted = scale.map_or_else(|| natural_unit(value.dim, system).from_si(value.value), |unit| unit.from_si(value.value));
    let mut formatted = format(value, scale, system, precision)?;
    formatted.number = number(converted, style);
    Ok(formatted)
}

fn fixed(value: f64, decimals: usize, zeros: bool) -> String {
    let out = format!("{value:.decimals$}");
    let out = if !zeros && decimals != 0 { out.trim_end_matches('0').trim_end_matches('.') } else { &out };
    if out.strip_prefix('-').is_some_and(|m| m.bytes().all(|c| matches!(c, b'0' | b'.'))) {
        return out[1..].to_owned();
    }
    out.to_owned()
}

fn number(value: f64, format: ResultFormat) -> String {
    let decimals = usize::from(format.decimals.min(15));
    if format.style == NumberStyle::Fraction {
        return fraction(value, format.accuracy.min(15), format.mixed_numbers);
    }
    let magnitude = value.abs();
    let threshold = 10_f64.powi(i32::from(format.exponential_threshold.min(15)));
    let exponential = match format.style {
        NumberStyle::Scientific | NumberStyle::Engineering => true,
        NumberStyle::General => magnitude != 0.0 && (magnitude >= threshold || magnitude < (10.0 / threshold).min(1.0)),
        _ => false,
    };
    if !exponential {
        return fixed(value, decimals, format.trailing_zeros);
    }

    // Rust's e format handles the full f64 range, including subnormals.
    let normalized = format!("{value:.17e}");
    let Some((mantissa, exponent)) = normalized.split_once('e') else { return normalized };
    let (Ok(mut mantissa), Ok(mut exponent)) = (mantissa.parse::<f64>(), exponent.parse::<i32>()) else { return normalized };
    let engineering = format.style == NumberStyle::Engineering || (format.style == NumberStyle::General && format.engineering_exponents);
    if engineering {
        let shift = exponent.rem_euclid(3);
        exponent -= shift;
        mantissa *= 10_f64.powi(shift);
    }
    let limit = if engineering { 1000.0 } else { 10.0 };
    let mut shown = fixed(mantissa, decimals, format.trailing_zeros);
    if shown.parse::<f64>().is_ok_and(|rounded| rounded.abs() >= limit) {
        shown = fixed(mantissa / limit, decimals, format.trailing_zeros);
        exponent += if engineering { 3 } else { 1 };
    }
    if format.e_notation { format!("{shown}E{exponent:+04}") } else { format!("{shown}×10^{exponent}") }
}

fn fraction(value: f64, accuracy: u8, mixed: bool) -> String {
    let magnitude = value.abs();
    if magnitude >= 1e12 || magnitude.fract() == 0.0 {
        return fixed(value, 0, false);
    }
    let whole = magnitude.trunc() as u64;
    let remainder = magnitude.fract();
    let target = 10_f64.powi(-i32::from(accuracy));
    let (mut prev_n, mut prev_d) = (1_u64, 0_u64);
    let (mut n, mut d) = (0_u64, 1_u64);
    let mut x = remainder.recip();
    for _ in 0..32 {
        if (n as f64 / d as f64 - remainder).abs() <= target || !x.is_finite() || x > 1_000_000.0 {
            break;
        }
        let a = x.floor() as u64;
        let next_d = a * d + prev_d;
        if next_d > 1_000_000 {
            break;
        }
        (prev_n, n) = (n, a * n + prev_n);
        (prev_d, d) = (d, next_d);
        x = x.fract().recip();
    }
    let sign = if value < 0.0 && (whole != 0 || n != 0) { "-" } else { "" };
    if d == 1 {
        return format!("{sign}{}", whole + n);
    }
    if mixed && whole != 0 { format!("{sign}{whole} {n}/{d}") } else { format!("{sign}{}/{d}", whole * d + n) }
}
