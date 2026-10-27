use super::export::{LINE_WIDTH, TABCOLSEP};
use super::limits::MAX_NEST;
use super::math::{balanced, closers};
use super::*;

/// Font size declarations (points, for the default 10 pt article).
pub(super) fn size_of(name: &str) -> Option<f32> {
    Some(match name {
        "tiny" => 5.0,
        "scriptsize" => 7.0,
        "footnotesize" => 8.0,
        "small" => 9.0,
        "normalsize" => 0.0,
        "large" => 12.0,
        "Large" => 14.4,
        "LARGE" => 17.28,
        "huge" => 20.74,
        "Huge" => 24.88,
        _ => return None,
    })
}

/// A named colour (`xcolor`'s base names).
fn named_color(name: &str) -> Option<Rgb> {
    Some(match name.trim() {
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(255, 255, 255),
        "red" => Rgb(255, 0, 0),
        "green" => Rgb(0, 255, 0),
        "blue" => Rgb(0, 0, 255),
        "cyan" => Rgb(0, 255, 255),
        "magenta" => Rgb(255, 0, 255),
        "yellow" => Rgb(255, 255, 0),
        "gray" => Rgb(128, 128, 128),
        "darkgray" => Rgb(64, 64, 64),
        "lightgray" => Rgb(191, 191, 191),
        "brown" => Rgb(191, 128, 64),
        "lime" => Rgb(191, 255, 0),
        "olive" => Rgb(128, 128, 0),
        "orange" => Rgb(255, 128, 0),
        "pink" => Rgb(255, 191, 191),
        "purple" => Rgb(191, 0, 64),
        "teal" => Rgb(0, 128, 128),
        "violet" => Rgb(128, 0, 128),
        _ => return None,
    })
}

/// `\textcolor[model]{spec}`: HTML, RGB (0–255) and rgb (0–1) models, or a name.
pub(super) fn parse_color(model: Option<&str>, spec: &str) -> Option<Rgb> {
    let spec = spec.trim();
    match model.map(str::trim) {
        Some("HTML") => crate::model::parse_color(&format!("#{spec}")),
        Some("RGB") => {
            let v: Vec<u8> = spec.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).map(|x| x.clamp(0.0, 255.0) as u8).collect();
            match v.as_slice() {
                [r, g, b] => Some(Rgb(*r, *g, *b)),
                _ => None,
            }
        }
        Some("rgb") => {
            let v: Vec<u8> = spec
                .split(',')
                .filter_map(|x| x.trim().parse::<f32>().ok())
                .filter(|x| x.is_finite())
                .map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            match v.as_slice() {
                [r, g, b] => Some(Rgb(*r, *g, *b)),
                _ => None,
            }
        }
        Some("gray") => spec.parse::<f32>().ok().filter(|x| x.is_finite()).map(|x| {
            let g = (x.clamp(0.0, 1.0) * 255.0).round() as u8;
            Rgb(g, g, g)
        }),
        _ => {
            // `red!50` (a tint) and `red!50!blue` (a mix) keep the first colour.
            named_color(spec.split('!').next().unwrap_or(spec))
        }
    }
}

/// A TeX length in points (`3cm`, `0.4\linewidth`), if it is one.
pub(super) fn length_pt(s: &str) -> Option<f32> {
    let s = s.trim().strip_prefix('+').unwrap_or(s.trim());
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: f32 = if n.is_empty() { 1.0 } else { n.parse().ok()? };
    let unit = unit.trim();
    // Macro names are case-sensitive (`\TEXTWIDTH` is not `\textwidth`); unit keywords are not.
    let unit = if unit.starts_with('\\') { unit.to_owned() } else { unit.to_ascii_lowercase() };
    let k = match unit.as_str() {
        "pt" => 1.0,
        "bp" => 72.27 / 72.0,
        "in" => 72.27,
        "cm" => 28.45,
        "mm" => 2.845,
        "em" => 10.0,
        "ex" => 4.3,
        "pc" => 12.0,
        "\\linewidth" | "\\textwidth" | "\\columnwidth" | "\\hsize" => LINE_WIDTH,
        _ => return None,
    };
    let v = n * k;
    (v.is_finite() && v > 0.0 && v < 10_000.0).then_some(v)
}

/// Column widths of a `tabular` column spec, when every column has one (`p{3cm}`).
pub(super) fn spec_widths(spec: &str) -> (usize, Vec<f32>) {
    let (cols, widths) = spec_widths_at(spec, 0);
    (cols.min(wordcraft_doc::table::MAX_COLS), if widths.len() == cols { widths } else { Vec::new() })
}

/// [`spec_widths`] at nesting `depth` of `*{n}{…}` repeats. The column count saturates and the
/// widths stop growing past [`wordcraft_doc::table::MAX_COLS`], so nested repeats can't overflow
/// or exhaust memory; the returned widths are empty unless every column has one.
fn spec_widths_at(spec: &str, depth: usize) -> (usize, Vec<f32>) {
    let max_cols = wordcraft_doc::table::MAX_COLS;
    let chars: Vec<char> = spec.chars().collect();
    let braces = closers(&chars, '{', '}');
    let mut cols = 0usize;
    let mut widths = Vec::new();
    let mut all = true;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            'l' | 'c' | 'r' | 'X' | 'S' | 'L' | 'C' | 'R' | 'J' => {
                cols = cols.saturating_add(1);
                all = false;
            }
            'p' | 'm' | 'b' => {
                cols = cols.saturating_add(1);
                match balanced(&chars, &braces, i + 1) {
                    Some((w, end)) => {
                        match length_pt(&w) {
                            Some(v) if widths.len() < max_cols => widths.push(v + 2.0 * TABCOLSEP),
                            Some(_) => {}
                            None => all = false,
                        }
                        i = end;
                        continue;
                    }
                    None => all = false,
                }
            }
            '*' => {
                // `*{3}{l}`: repeat. Past the nesting limit the repeat is skipped, unmeasured.
                if let Some((n, end)) = balanced(&chars, &braces, i + 1)
                    && let Some((inner, end2)) = balanced(&chars, &braces, end)
                {
                    if depth < MAX_NEST {
                        let n: usize = n.trim().parse().unwrap_or(1).min(64);
                        let (c2, w2) = spec_widths_at(&inner, depth + 1);
                        cols = cols.saturating_add(c2.saturating_mul(n));
                        if w2.len() == c2 {
                            for _ in 0..n {
                                if widths.len() >= max_cols {
                                    break;
                                }
                                widths.extend(w2.iter().take(max_cols - widths.len()));
                            }
                        } else {
                            all = false;
                        }
                    } else {
                        all = false;
                    }
                    i = end2;
                    continue;
                }
            }
            '@' | '!' | '>' | '<' => {
                // `@{…}`, `>{…}`: skip the argument.
                if let Some((_, end)) = balanced(&chars, &braces, i + 1) {
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (cols, if all && widths.len() == cols { widths } else { Vec::new() })
}
