use super::*;

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

/// Parse a CSS / HTML colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(...)`, `hsl(...)` or a basic name.
pub fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        if (h.len() == 3 || h.len() == 4) && h.is_ascii() {
            let d = |i: usize| h.get(i..i + 1).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| v * 17);
            return Some(Rgb(d(0)?, d(1)?, d(2)?));
        }
        if h.len() == 8 && h.is_ascii() {
            let p = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
            return Some(Rgb(p(0)?, p(2)?, p(4)?));
        }
        return Rgb::parse(h);
    }
    if let Some(inner) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let inner = inner.trim_end_matches(')').trim();
        let before_slash = inner.split('/').next().unwrap_or(inner);
        let parts: Vec<&str> = if before_slash.contains(',') { before_slash.split(',').collect() } else { before_slash.split_whitespace().collect() };
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
            return Some(Rgb(r, g, b));
        }
        return None;
    }
    if let Some(inner) = s.strip_prefix("hsl(").or_else(|| s.strip_prefix("hsla(")) {
        let inner = inner.trim_end_matches(')').trim();
        let before_slash = inner.split('/').next().unwrap_or(inner);
        let parts: Vec<&str> = if before_slash.contains(',') { before_slash.split(',').collect() } else { before_slash.split_whitespace().collect() };
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
        return Some(hsl_to_rgb(h, s, l));
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
