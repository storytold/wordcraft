use super::{Align, Fmt, model, parse_color};

/// Consume an escape after its backslash, retaining the source spelling for the colour reader.
/// Decoding is only used to recognise URL identifiers, not to add new colour syntax.
fn background_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, raw: &mut String) -> Option<char> {
    let first = chars.next()?;
    raw.push(first);
    if !first.is_ascii_hexdigit() {
        return Some(first);
    }
    let mut code = first.to_digit(16).unwrap_or(0);
    for _ in 1..6 {
        let Some(c) = chars.peek().copied().filter(char::is_ascii_hexdigit) else { break };
        chars.next();
        raw.push(c);
        code = code * 16 + c.to_digit(16).unwrap_or(0);
    }
    if let Some(c) = chars.peek().copied().filter(char::is_ascii_whitespace) {
        chars.next();
        raw.push(c);
        if c == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
            raw.push('\n');
        }
    }
    Some(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{fffd}'))
}

/// Find a supported background colour without splitting function arguments or quoted strings.
/// Images and gradients remain unsupported; their contents aren't separate colour values.
fn background_color(value: &str) -> Option<wordcraft_doc::Rgb> {
    let mut depth = 0usize;
    let mut url_depth = None;
    let mut quote = None;
    let mut token = String::new();
    let mut name = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            token.push(c);
            let decoded = background_escape(&mut chars, &mut token);
            if quote.is_none()
                && let Some(c) = decoded
            {
                name.push(c);
            }
            continue;
        }
        // Comments separate tokens but aren't syntax, even inside a colour function.
        // A quoted URL can contain the same characters literally.
        if quote.is_none() && url_depth.is_none() && c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut star = false;
            for c in chars.by_ref() {
                if star && c == '/' {
                    break;
                }
                star = c == '*';
            }
            if depth == 0 {
                if let Some(color) = parse_color(&token) {
                    return Some(color);
                }
                token.clear();
            } else {
                token.push(' ');
            }
            name.clear();
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else {
            match c {
                '\'' | '"' => {
                    quote = Some(c);
                    name.clear();
                }
                '(' => {
                    depth = depth.saturating_add(1);
                    if name.eq_ignore_ascii_case("url") {
                        // Even an unquoted URL keeps literal /* bytes (CSS Syntax §4.3.6).
                        url_depth = Some(depth);
                    }
                    name.clear();
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    if url_depth.is_some_and(|start| depth < start) {
                        url_depth = None;
                    }
                    name.clear();
                }
                _ if depth == 0 && c.is_ascii_whitespace() => {
                    if let Some(color) = parse_color(&token) {
                        return Some(color);
                    }
                    token.clear();
                    name.clear();
                    continue;
                }
                _ if c.is_alphanumeric() || c >= '\u{80}' || matches!(c, '_' | '-') => name.push(c),
                _ => name.clear(),
            }
        }
        token.push(c);
    }
    parse_color(&token)
}

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
                if let Some(c) = background_color(v) {
                    f.background = Some(c);
                }
            }
            "font-size" => {
                if let Some(p) = css_len_pt(v).filter(|p| *p >= 1.0 && *p <= 1638.0) {
                    f.size = Some(p);
                }
            }
            "font-family" => {
                let fam = v.split(',').next().unwrap_or("").trim().trim_matches(['"', '\'']).to_string();
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
