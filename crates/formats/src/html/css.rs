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

/// Whether a colour states alpha 0 (`rgba(0, 0, 0, 0)`, `hsl(0 0% 0% / 0%)`, `#00000000`): what a
/// browser writes for a run with no background. A partly transparent colour (alpha above 0) is
/// kept, as the opaque colour.
fn fully_transparent(color: &str) -> bool {
    let c = color.trim().to_ascii_lowercase();
    if let Some(h) = c.strip_prefix('#') {
        return (h.len() == 4 && h.ends_with('0')) || (h.len() == 8 && h.ends_with("00"));
    }
    // Only the first colour function: `rgba(0, 0, 0, 0) none repeat` is a shorthand around it.
    let call = c.split_inclusive(')').next().unwrap_or("");
    let Some((_, inner)) = call.split_once('(') else { return false };
    let inner = inner.trim_end_matches(')');
    let alpha = match inner.split_once('/') {
        Some((_, a)) => Some(a),
        None => inner.split(',').nth(3),
    };
    alpha.is_some_and(|a| {
        let a = a.trim();
        a.strip_suffix('%').unwrap_or(a).trim().parse::<f32>().is_ok_and(|x| x == 0.0)
    })
}

/// The colour in a `background` value: the whole value, else an `rgb()` / `rgba()` call inside a
/// shorthand (`rgb(255, 255, 0) repeat-x`, whose spaces would split it apart), else the first
/// word that is a colour. A fully transparent colour is no colour.
fn background_color(v: &str) -> Option<wordcraft_doc::Rgb> {
    if fully_transparent(v) {
        return None;
    }
    parse_color(v).or_else(|| {
        let lower = v.to_ascii_lowercase();
        for call in ["rgba(", "rgb("] {
            if let Some(i) = lower.find(call)
                && let Some(len) = lower.get(i..).and_then(|t| t.find(')'))
                && let Some(text) = v.get(i..=i + len)
            {
                return if fully_transparent(text) { None } else { parse_color(text) };
            }
        }
        v.split_whitespace().find_map(|w| if fully_transparent(w) { None } else { parse_color(w) })
    })
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
                // The whole value first: browsers write `rgb(255, 255, 0)`, whose spaces would
                // split a function apart. A shorthand (`#ff0 url(…) no-repeat`) goes word by word.
                if let Some(c) = background_color(v) {
                    f.background = Some(c);
                }
            }
            "font-size" => {
                if let Some(p) = css_len_pt(v).filter(|p| *p >= 1.0 && *p <= 1638.0) {
                    f.size = Some(p);
                }
            }
            // The shorthand (`bold italic 22px/1.2 "Courier New", monospace`) resets style, weight,
            // size and family together: read it as the longhands it stands for.
            "font" => {
                let (mut weight, mut style, mut size) = ("normal", "normal", None);
                let mut rest = lv.as_str();
                let mut family = "";
                while let Some(tok) = rest.split_whitespace().next() {
                    let after = rest.trim_start().strip_prefix(tok).unwrap_or("");
                    let head = tok.split('/').next().unwrap_or(tok);
                    match tok {
                        "italic" | "oblique" => style = "italic",
                        "bold" | "bolder" => weight = "bold",
                        "normal" | "lighter" | "small-caps" => {}
                        _ if tok.parse::<u32>().is_ok_and(|w| (1..=1000).contains(&w)) => {
                            weight = if tok.parse::<u32>().is_ok_and(|w| w >= 600) { "bold" } else { "normal" };
                        }
                        _ if css_len_pt(head).is_some() => {
                            size = css_len_pt(head);
                            // An optional `/ line-height`, with or without spaces around the slash.
                            let mut tail = after.trim_start();
                            fn drop_word(t: &str) -> &str {
                                t.trim_start().split_once(char::is_whitespace).map_or("", |(_, r)| r)
                            }
                            if tok.ends_with('/') {
                                tail = drop_word(tail);
                            } else if !tok.contains('/')
                                && let Some(r) = tail.strip_prefix('/')
                            {
                                tail = drop_word(r);
                            }
                            family = tail.trim();
                            break;
                        }
                        _ => break,
                    }
                    rest = after;
                }
                // The family is required: `font: 12pt` and `font: 12pt inherit` are invalid, and an
                // invalid declaration changes nothing.
                let family_ok = !family.is_empty() && !matches!(family, "inherit" | "initial" | "unset" | "revert");
                if let Some(pt) = size.filter(|_| family_ok) {
                    // The shorthand sets the family too: an outer span's font or code styling goes.
                    f.code = false;
                    f.font = None;
                    // Keep the family's original case (the match above was on the lowercased text;
                    // `lv` is ASCII-lowercased, so its byte offsets are `v`'s).
                    let at = v.len().saturating_sub(family.len());
                    let family = v.get(at..).unwrap_or("");
                    let css = format!("font-weight:{weight};font-style:{style};font-size:{pt}pt;font-family:{family}");
                    apply_style(&css, f, align, preserve, page_break, page_break_after);
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
