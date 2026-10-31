use super::{Align, Fmt, model, parse_color};

/// A CSS length (or font-size keyword) in points; `None` when it is not one.
pub fn css_len_pt(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(x) = v.strip_suffix("pt") {
        num(x)
    } else if let Some(x) = v.strip_suffix("px") {
        num(x).map(|p| p * 0.75)
    } else if let Some(x) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        num(x).map(|p| p * 12.0)
    } else if let Some(x) = v.strip_suffix("in") {
        num(x).map(|p| p * 72.0)
    } else if let Some(x) = v.strip_suffix("cm") {
        num(x).map(|p| p * 72.0 / 2.54)
    } else if let Some(x) = v.strip_suffix("mm") {
        num(x).map(|p| p * 72.0 / 25.4)
    } else if let Some(x) = v.strip_suffix('%') {
        num(x).map(|p| p * 0.12)
    } else {
        match v.as_str() {
            "xx-small" => Some(7.0),
            "x-small" => Some(7.5),
            "small" | "smaller" => Some(10.0),
            "medium" => Some(12.0),
            "large" | "larger" => Some(13.5),
            "x-large" => Some(18.0),
            "xx-large" => Some(24.0),
            "xxx-large" => Some(36.0),
            _ => num(&v).map(|p| p * 0.75),
        }
    }
}

fn strip_important(v: &str) -> &str {
    let s = v.trim();
    if let Some(pos) = s.rfind('!') {
        let after = s[pos + 1..].trim();
        if after.eq_ignore_ascii_case("important") {
            return s[..pos].trim();
        }
    }
    s
}

/// Decode CSS escapes (CSS Syntax §4.3.7): `\` and one to six hex digits, with one whitespace
/// after them, is that code point; `\` and any other character is that character. CR LF, CR and
/// form feed count as one newline, as CSS reads them (§3.3).
pub(super) fn css_unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    // Takes a newline (CR LF as one) or other whitespace off the front, if there is one.
    let skip_space = |it: &mut std::iter::Peekable<std::str::Chars>| match it.peek() {
        Some('\r') => {
            it.next();
            it.next_if_eq(&'\n');
        }
        Some(' ' | '\t' | '\n' | '\u{C}') => {
            it.next();
        }
        _ => {}
    };
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && it.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.extend(it.next());
        }
        if hex.is_empty() {
            // Before a newline it continues the line; at the end it is dropped.
            match it.peek() {
                Some('\r' | '\n' | '\u{C}') => skip_space(&mut it),
                Some(_) => o.extend(it.next()),
                None => {}
            }
            continue;
        }
        skip_space(&mut it);
        let cp = u32::from_str_radix(&hex, 16).unwrap_or(0);
        o.push(char::from_u32(cp).filter(|c| *c != '\0').unwrap_or('\u{FFFD}'));
    }
    o
}

/// Apply a `style` attribute to formatting / block properties.
pub(super) fn apply_style(
    css: &str,
    f: &mut Fmt,
    align: &mut Option<Align>,
    preserve: &mut bool,
    page_break: &mut bool,
    page_break_after: &mut bool,
) {
    for decl in css.split(';').take(64) {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = strip_important(v);
        let lv = v.to_ascii_lowercase();
        match k.as_str() {
            "font-weight" => f.bold = lv.starts_with("bold") || lv.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => f.italic = lv == "italic" || lv == "oblique",
            "text-decoration" | "text-decoration-line" => {
                if lv.contains("none") {
                    f.underline = false;
                    f.strike = false;
                }
                if lv.contains("underline") {
                    f.underline = true;
                }
                if lv.contains("line-through") {
                    f.strike = true;
                }
            }
            "color" => {
                if let Some(c) = parse_color(v) {
                    f.color = Some(c);
                }
            }
            "background-color" | "background" => {
                if let Some(c) = v.split_whitespace().find_map(parse_color) {
                    f.background = Some(c);
                }
            }
            "font-size" => {
                if let Some(p) = css_len_pt(v).filter(|p| *p >= 1.0 && *p <= 1638.0) {
                    f.size = Some(p);
                }
            }
            "font-family" => {
                let fam = css_unescape(v.split(',').next().unwrap_or("").trim().trim_matches(['"', '\'']));
                if model::is_mono(&fam) || fam.eq_ignore_ascii_case("monospace") {
                    f.code = true;
                } else if !fam.is_empty() && !matches!(fam.to_ascii_lowercase().as_str(), "serif" | "sans-serif" | "inherit" | "initial") {
                    f.font = model::kept_font(&fam).or(f.font.take());
                }
            }
            "vertical-align" => {
                f.sup = lv == "super";
                f.sub = lv == "sub";
            }
            "text-align" => {
                *align = match lv.as_str() {
                    "center" => Some(Align::Center),
                    "right" | "end" => Some(Align::Right),
                    "justify" => Some(Align::Justify),
                    "left" | "start" => Some(Align::Left),
                    _ => *align,
                }
            }
            "white-space" => *preserve = lv.starts_with("pre"),
            "page-break-before" | "break-before" => *page_break = lv == "always" || lv == "page",
            "page-break-after" | "break-after" => *page_break_after = lv == "always" || lv == "page",
            _ => {}
        }
    }
}
