use super::Rgb;

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Rgb {
    let h = (h / 360.0).rem_euclid(1.0);
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    if s == 0.0 {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return Rgb(v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue_to_rgb = |mut t: f64| -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    };
    let r = (hue_to_rgb(h + 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (hue_to_rgb(h) * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (hue_to_rgb(h - 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(r, g, b)
}

/// Composite `rgb` over the white page a paste lands on, at CSS alpha `a` (0..1): `rgba(0, 0, 0,
/// 0.45)` reads like the rgb(140, 140, 140) it looks like on the site, instead of solid black.
fn over_white(r: u8, g: u8, b: u8, a: f32) -> Rgb {
    let mix = |c: u8| (c as f32 * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    Rgb(mix(r), mix(g), mix(b))
}

/// The alpha of a CSS colour's `/ alpha` or 4th-value part, where present: 1.0 (opaque) otherwise.
fn css_alpha(after: Option<&str>) -> f32 {
    let Some(a) = after else { return 1.0 };
    let a = a.trim();
    // CSS Colour 4's `none` is a missing component, and a missing alpha is transparent.
    if a.eq_ignore_ascii_case("none") {
        return 0.0;
    }
    // A value that is not a finite number (`nan`, `inf`) is no alpha at all: clamp would keep
    // NaN, and `over_white` would turn the colour black.
    let parse = |t: &str| t.parse::<f32>().ok().filter(|f| f.is_finite());
    if let Some(p) = a.strip_suffix('%') {
        return parse(p.trim()).map(|f| f.clamp(0.0, 100.0) / 100.0).unwrap_or(1.0);
    }
    parse(a).map(|f| f.clamp(0.0, 1.0)).unwrap_or(1.0)
}

/// Parse a CSS / HTML colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(...)`, `hsl(...)` or a basic name.
/// A colour with alpha is composited over white: the model keeps opaque colours, and a paste lands
/// on a white page.
pub fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        if (h.len() == 3 || h.len() == 4) && h.is_ascii() {
            let d = |i: usize| h.get(i..i + 1).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| v * 17);
            let (r, g, b) = (d(0)?, d(1)?, d(2)?);
            // A 4th digit is the alpha, composited over the page like `rgba(...)`'s is.
            let a = if h.len() == 4 { d(3)? as f32 / 255.0 } else { 1.0 };
            return Some(over_white(r, g, b, a));
        }
        if h.len() == 8 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
            let p = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
            return Some(over_white(p(0)?, p(2)?, p(4)?, p(6)? as f32 / 255.0));
        }
        return Rgb::parse(h);
    }
    if let Some(inner) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let inner = inner.trim_end_matches(')').trim();
        let (rgb_part, alpha_part) = inner.split_once('/').map_or((inner, None), |(a, b)| (a, Some(b)));
        // Legacy comma syntax carries the alpha as a 4th value (`rgba(0, 0, 0, 0.45)`).
        let parts: Vec<&str> = if rgb_part.contains(',') { rgb_part.split(',').collect() } else { rgb_part.split_whitespace().collect() };
        let alpha = match alpha_part {
            Some(a) => css_alpha(Some(a)),
            None if parts.len() >= 4 => css_alpha(Some(parts[3])),
            _ => 1.0,
        };
        let v: Vec<u8> = parts
            .iter()
            .take(3)
            .filter_map(|x| {
                let x = x.trim();
                if let Some(p) = x.strip_suffix('%') {
                    p.trim().parse::<f32>().ok().map(|f| (f.clamp(0.0, 100.0) * 2.55).round() as u8)
                } else {
                    x.parse::<f32>().ok().map(|f| f.clamp(0.0, 255.0).round() as u8)
                }
            })
            .collect();
        if let [r, g, b] = v[..] {
            return Some(over_white(r, g, b, alpha));
        }
        return None;
    }
    if let Some(inner) = s.strip_prefix("hsl(").or_else(|| s.strip_prefix("hsla(")) {
        let inner = inner.trim_end_matches(')').trim();
        let (hsl_part, alpha_part) = inner.split_once('/').map_or((inner, None), |(a, b)| (a, Some(b)));
        let parts: Vec<&str> = if hsl_part.contains(',') { hsl_part.split(',').collect() } else { hsl_part.split_whitespace().collect() };
        let alpha = match alpha_part {
            Some(a) => css_alpha(Some(a)),
            None if parts.len() >= 4 => css_alpha(Some(parts[3])),
            _ => 1.0,
        };
        if parts.len() < 3 {
            return None;
        }
        let h_str = parts[0].trim();
        let h: f64 = if let Some(deg) = h_str.strip_suffix("deg") {
            deg.trim().parse().ok()?
        } else if let Some(rad) = h_str.strip_suffix("rad") {
            rad.trim().parse::<f64>().ok()? * 180.0 / std::f64::consts::PI
        } else if let Some(turn) = h_str.strip_suffix("turn") {
            turn.trim().parse::<f64>().ok()? * 360.0
        } else {
            h_str.parse().ok()?
        };
        let parse_pct = |x: &str| -> Option<f64> {
            let x = x.trim();
            if let Some(p) = x.strip_suffix('%') {
                Some((p.trim().parse::<f64>().ok()? / 100.0).clamp(0.0, 1.0))
            } else {
                Some(x.parse::<f64>().ok()?.clamp(0.0, 1.0))
            }
        };
        let s = parse_pct(parts[1])?;
        let l = parse_pct(parts[2])?;
        let Rgb(r, g, b) = hsl_to_rgb(h, s, l);
        return Some(over_white(r, g, b, alpha));
    }
    Some(match s.as_str() {
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(255, 255, 255),
        "red" => Rgb(255, 0, 0),
        "green" => Rgb(0, 128, 0),
        "lime" => Rgb(0, 255, 0),
        "blue" => Rgb(0, 0, 255),
        "yellow" => Rgb(255, 255, 0),
        "cyan" | "aqua" => Rgb(0, 255, 255),
        "magenta" | "fuchsia" => Rgb(255, 0, 255),
        "gray" | "grey" => Rgb(128, 128, 128),
        "silver" => Rgb(192, 192, 192),
        "maroon" => Rgb(128, 0, 0),
        "olive" => Rgb(128, 128, 0),
        "navy" => Rgb(0, 0, 128),
        "purple" => Rgb(128, 0, 128),
        "teal" => Rgb(0, 128, 128),
        "orange" => Rgb(255, 165, 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_digit_hex_rejects_signs() {
        assert_eq!(parse_color("#+1+2+3+4"), None);
        assert_eq!(parse_color("#11223344"), Some(Rgb(0x11, 0x22, 0x33)));
    }
}
