//! Character formatting (Home › Font).

use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::props::{Border, CharProps, Highlight, Rgb, TextColor, Underline, VertAlign};
use wordcraft_doc::resolve::ResolvedChar;

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Word's font size list.
pub const SIZES: [f32; 17] = [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("format.bold", "Bold", "Home › Font", |s, v| {
            // Like Word's button, for complex-script (Persian, Arabic) text too: an unset `bold_cs`
            // follows `bold`.
            toggle(
                s,
                v,
                |r| r.bold,
                |c, on| {
                    c.bold = Some(on);
                    c.bold_cs = None;
                },
            )
        })
        .key("Mod+B")
        .params(r#"{"value"?: bool}"#),
        CommandSpec::new("format.italic", "Italic", "Home › Font", |s, v| {
            toggle(
                s,
                v,
                |r| r.italic,
                |c, on| {
                    c.italic = Some(on);
                    c.italic_cs = None;
                },
            )
        })
        .key("Mod+I")
        .params(r#"{"value"?: bool}"#),
        CommandSpec::new("format.underline", "Underline", "Home › Font", underline)
            .key("Mod+U")
            .params(r#"{"value"?: bool, "style"?: "single|double|thick|dotted|dash|dotDash|dotDotDash|wave|words"}"#),
        CommandSpec::new("format.doubleUnderline", "Double Underline", "Home › Font › Underline", |s, _| {
            underline(s, &json!({"style": "double"}))
        })
        .key("Mod+Shift+D"),
        CommandSpec::new("format.wordUnderline", "Underline Words Only", "Home › Font › Underline", |s, _| {
            underline(s, &json!({"style": "words"}))
        })
        .key("Mod+Shift+W"),
        CommandSpec::new("format.strikethrough", "Strikethrough", "Home › Font", |s, v| toggle(s, v, |r| r.strike, |c, on| c.strike = Some(on)))
            .params(r#"{"value"?: bool}"#),
        // Note: off clears the direct property only; a border inherited from a character style stays on.
        CommandSpec::new("format.border", "Character Border", "Home › Font", |s, v| {
            toggle(s, v, |r| r.border.is_some(), |c, on| c.border = on.then(|| Border::single(0.5)))
        })
        .params(r#"{"value"?: bool}"#),
        CommandSpec::new("format.doubleStrikethrough", "Double Strikethrough", "Home › Font › Font", |s, v| {
            toggle(s, v, |r| r.double_strike, |c, on| c.double_strike = Some(on))
        }),
        CommandSpec::new("format.subscript", "Subscript", "Home › Font", |s, v| {
            toggle(
                s,
                v,
                |r| r.vert_align == VertAlign::Subscript,
                |c, on| c.vert_align = Some(if on { VertAlign::Subscript } else { VertAlign::Baseline }),
            )
        })
        .key("Mod+="),
        CommandSpec::new("format.superscript", "Superscript", "Home › Font", |s, v| {
            toggle(
                s,
                v,
                |r| r.vert_align == VertAlign::Superscript,
                |c, on| c.vert_align = Some(if on { VertAlign::Superscript } else { VertAlign::Baseline }),
            )
        })
        .key("Mod+Shift+="),
        CommandSpec::new("format.allCaps", "All Caps", "Home › Font › Font", |s, v| toggle(s, v, |r| r.caps, |c, on| c.caps = Some(on)))
            .key("Mod+Shift+A"),
        CommandSpec::new("format.smallCaps", "Small Caps", "Home › Font › Font", |s, v| {
            toggle(s, v, |r| r.small_caps, |c, on| c.small_caps = Some(on))
        })
        .key("Mod+Shift+K"),
        CommandSpec::new("format.hidden", "Hidden", "Home › Font › Font", |s, v| toggle(s, v, |r| r.hidden, |c, on| c.hidden = Some(on)))
            .key("Mod+Shift+H"),
        CommandSpec::new("format.outline", "Outline", "Home › Font › Text Effects", |s, v| {
            toggle(s, v, |r| r.outline, |c, on| c.outline = Some(on))
        }),
        CommandSpec::new("format.shadow", "Shadow", "Home › Font › Text Effects", |s, v| toggle(s, v, |r| r.shadow, |c, on| c.shadow = Some(on))),
        CommandSpec::new("format.emboss", "Emboss", "Home › Font › Font", |s, v| toggle(s, v, |r| r.emboss, |c, on| c.emboss = Some(on))),
        CommandSpec::new("format.engrave", "Engrave", "Home › Font › Font", |s, v| toggle(s, v, |r| r.engrave, |c, on| c.engrave = Some(on))),
        CommandSpec::new("format.font", "Font", "Home › Font", font).key("Mod+Shift+F").params(r#"{"name": string}"#),
        CommandSpec::new("format.size", "Font Size", "Home › Font", size).key("Mod+Shift+P").params(r#"{"size": number}"#),
        CommandSpec::new("format.growFont", "Increase Font Size", "Home › Font", |s, _| step_size(s, 1)).key("Mod+Shift+. / Mod+Shift+>"),
        CommandSpec::new("format.shrinkFont", "Decrease Font Size", "Home › Font", |s, _| step_size(s, -1)).key("Mod+Shift+, / Mod+Shift+<"),
        CommandSpec::new("format.growFont1", "Grow Font 1 Point", "Home › Font", |s, _| nudge_size(s, 1.0)).key("Mod+]"),
        CommandSpec::new("format.shrinkFont1", "Shrink Font 1 Point", "Home › Font", |s, _| nudge_size(s, -1.0)).key("Mod+["),
        CommandSpec::new("format.color", "Font Color", "Home › Font", color).params(r#"{"color": "RRGGBB" | "auto"}"#),
        CommandSpec::new("format.highlight", "Text Highlight Color", "Home › Font", highlight).params(
            r#"{"color": "yellow|brightGreen|turquoise|pink|blue|red|darkBlue|teal|green|violet|darkRed|darkYellow|gray50|gray25|black|none"}"#,
        ),
        CommandSpec::new("format.shading", "Character Shading", "Home › Font", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            apply(s, &|x| x.shading = c)
        })
        .params(r#"{"color": "RRGGBB" | null}"#),
        CommandSpec::new("format.changeCase", "Change Case", "Home › Font", change_case)
            .key("Shift+F3")
            .params(r#"{"mode"?: "sentence|lower|upper|title|toggle"}"#),
        CommandSpec::new("format.clear", "Clear All Formatting", "Home › Font", clear).key("Mod+Space"),
        CommandSpec::new("format.spacing", "Character Spacing", "Home › Font › Font › Advanced", |s, v| {
            let x = p::req_f32(v, "points")?.clamp(-100.0, 100.0);
            apply(s, &|c| c.spacing = Some(x))
        })
        .params(r#"{"points": number}"#),
        CommandSpec::new("format.scale", "Character Scale", "Home › Font › Font › Advanced", |s, v| {
            let x = p::req_f32(v, "percent")?.clamp(1.0, 600.0);
            apply(s, &|c| c.scale = Some(x))
        })
        .params(r#"{"percent": number}"#),
        CommandSpec::new("format.position", "Character Position", "Home › Font › Font › Advanced", |s, v| {
            let x = p::req_f32(v, "points")?.clamp(-1584.0, 1584.0);
            apply(s, &|c| c.position = Some(x))
        })
        .params(r#"{"points": number}"#),
        CommandSpec::new("format.charStyle", "Apply Character Style", "Home › Styles", char_style).params(r#"{"style": string}"#),
        CommandSpec::new("format.set", "Set Character Formatting", "Home › Font › Font", set).params(r#"{"props": CharProps}"#),
        CommandSpec::new("format.fontDialog", "Font Dialog", "Home › Font", |s, _| {
            s.ui_requests.push(json!({"open": "font"}));
            sel_result(s)
        })
        .key("Mod+D")
        .pure(),
        CommandSpec::new("format.state", "Formatting at Selection", "Home › Font", |s, _| Ok(state(s))).pure(),
    ]
}

/// The selection to format: the selection, or the word around a collapsed caret (Word
/// formats the whole word when the caret is inside one). `None` = caret between words.
fn target(s: &Session) -> Option<(Pos, Pos)> {
    let (a, b) = s.sel.ordered();
    if a != b {
        return Some((a, b));
    }
    let p = s.doc.para_at(&a)?;
    let is_w = |c: char| c.is_alphanumeric() || c == '\'' || c == '_';
    let before = p.text.get(..a.off)?.chars().next_back();
    let after = p.text.get(a.off..)?.chars().next();
    if before.is_some_and(is_w) && after.is_some_and(is_w) {
        let (x, y) = p.word_at(a.off);
        // word_at includes trailing spaces; Word formats just the word.
        let word_end = p.text.get(x..y).map(|w| x + w.trim_end().len()).unwrap_or(y);
        return Some((Pos { off: x, ..a.clone() }, Pos { off: word_end, ..a }));
    }
    None
}

/// The non-empty rows of a column selection, if one is active.
fn column_rows(s: &Session) -> Option<Vec<(Pos, Pos)>> {
    let segs = s.column_segments()?;
    let rows: Vec<(Pos, Pos)> = segs
        .iter()
        .map(|(a, b)| (s.doc.clamp(a), s.doc.clamp(b)))
        .filter(|(a, b)| a != b && a.path == b.path && a.story == b.story)
        .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
        .collect();
    // An empty block (column mode just started) formats like a caret.
    (!rows.is_empty()).then_some(rows)
}

/// Apply a change to the selection (or the pending caret formatting).
pub fn apply(s: &mut Session, f: &dyn Fn(&mut CharProps)) -> CmdResult {
    // A column selection formats each row's piece.
    if let Some(rows) = column_rows(s) {
        for (a, b) in rows {
            s.doc.format_range(&a, &b, f)?;
        }
        return sel_result(s);
    }
    match target(s) {
        Some((a, b)) => {
            s.doc.format_range(&a, &b, f)?;
        }
        None => {
            let mut pend = s.typing_props();
            f(&mut pend);
            s.pending = Some(pend);
            // An empty paragraph's mark takes the formatting too (so it shows the new size).
            let fpos = s.sel.focus.clone();
            if s.doc.para_at(&fpos).is_some_and(|p| p.is_empty())
                && let Ok(para) = s.doc.para_mut(fpos.story, &fpos.path)
            {
                f(&mut para.mark);
                para.touch();
            }
        }
    }
    sel_result(s)
}

/// Resolved formatting of every character in the target (or the caret).
fn resolved(s: &Session) -> Vec<ResolvedChar> {
    let mut out = Vec::new();
    if let Some(rows) = column_rows(s) {
        for (a, b) in rows {
            let Some(p) = s.doc.para_at(&a) else { continue };
            for (r, c) in p.run_ranges() {
                if r.end > a.off && r.start < b.off {
                    out.push(s.doc.styles.resolve_char(p.props.style.as_deref(), c));
                }
            }
            if out.len() > 10_000 {
                break;
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    let Some((a, b)) = target(s) else {
        let f = &s.sel.focus;
        let style = s.doc.para_at(f).and_then(|p| p.props.style.clone());
        out.push(s.doc.styles.resolve_char(style.as_deref(), &s.typing_props()));
        return out;
    };
    for path in s.doc.paths_between(&a, &b) {
        let Some(p) = s.doc.para(a.story, &path) else { continue };
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { p.len() };
        for (r, c) in p.run_ranges() {
            if r.end > from && r.start < to {
                out.push(s.doc.styles.resolve_char(p.props.style.as_deref(), c));
            }
        }
        if out.len() > 10_000 {
            break;
        }
    }
    if out.is_empty() {
        let style = s.doc.para_at(&a).and_then(|p| p.props.style.clone());
        out.push(s.doc.styles.resolve_char(style.as_deref(), &s.typing_props()));
    }
    out
}

fn toggle(s: &mut Session, v: &Value, get: fn(&ResolvedChar) -> bool, set: fn(&mut CharProps, bool)) -> CmdResult {
    let on = match p::bool(v, "value") {
        Some(b) => b,
        None => !resolved(s).iter().all(get),
    };
    apply(s, &|c| set(c, on))
}

fn underline(s: &mut Session, v: &Value) -> CmdResult {
    let style = p::str(v, "style").map(Underline::from_ooxml);
    let on = match (p::bool(v, "value"), style) {
        (Some(b), _) => b,
        (None, Some(_)) => true,
        (None, None) => !resolved(s).iter().all(|r| r.underline != Underline::None),
    };
    let u = if on { style.unwrap_or(Underline::Single) } else { Underline::None };
    apply(s, &|c| c.underline = Some(u))
}

fn font(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    if name.is_empty() || name.len() > 128 {
        return Err(CmdError::Params("bad font name".into()));
    }
    // The font applies to Persian/Arabic text in the selection too (its complex-script font).
    apply(s, &|c| {
        c.font = Some(name.clone());
        c.font_cs = Some(name.clone());
    })
}

fn size(s: &mut Session, v: &Value) -> CmdResult {
    let sz = p::req_f32(v, "size")?;
    if !(1.0..=1638.0).contains(&sz) {
        return Err(CmdError::Params("size must be between 1 and 1638".into()));
    }
    // Word rounds to half points.
    let sz = (sz * 2.0).round() / 2.0;
    apply(s, &|c| {
        c.size = Some(sz);
        c.size_cs = None;
    })
}

fn current_size(s: &Session) -> f32 {
    resolved(s).first().map(|r| r.size).unwrap_or(12.0)
}

fn step_size(s: &mut Session, dir: i32) -> CmdResult {
    let cur = current_size(s);
    let next = if dir > 0 {
        SIZES.iter().copied().find(|x| *x > cur + 0.01).unwrap_or(((cur / 10.0).floor() + 1.0) * 10.0)
    } else {
        SIZES.iter().rev().copied().find(|x| *x < cur - 0.01).unwrap_or(1.0)
    };
    let next = next.clamp(1.0, 1638.0);
    // Each run steps from its own size when the selection mixes sizes.
    apply(s, &|c| {
        c.size = Some(next);
        c.size_cs = None;
    })
}

fn nudge_size(s: &mut Session, d: f32) -> CmdResult {
    let next = (current_size(s) + d).clamp(1.0, 1638.0);
    apply(s, &|c| {
        c.size = Some(next);
        c.size_cs = None;
    })
}

fn color(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::req_str(v, "color")?;
    let tc = if c.eq_ignore_ascii_case("auto") || c.eq_ignore_ascii_case("automatic") {
        TextColor::Auto
    } else {
        TextColor::Rgb(Rgb::parse(c).ok_or_else(|| CmdError::Params("color must be RRGGBB or auto".into()))?)
    };
    apply(s, &|x| x.color = Some(tc))
}

fn highlight(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::str(v, "color").unwrap_or("yellow");
    let h = Highlight::ALL
        .iter()
        .copied()
        .find(|h| {
            h.ooxml().eq_ignore_ascii_case(c)
                || h.name().replace(['-', ' ', '%'], "").eq_ignore_ascii_case(&c.replace(['-', ' ', '%'], ""))
                || format!("{h:?}").eq_ignore_ascii_case(c)
        })
        .ok_or_else(|| CmdError::Params(format!("unknown highlight colour `{c}`")))?;
    apply(s, &|x| x.highlight = Some(h))
}

fn change_case(s: &mut Session, v: &Value) -> CmdResult {
    let (a, b) = s.sel.ordered();
    let (a, b) = if a == b { target(s).unwrap_or((a, b)) } else { (a, b) };
    if a == b {
        return sel_result(s);
    }
    let text = s.doc.copy_range(&a, &b).plain_text();
    let mode = p::str(v, "mode").map(str::to_string).unwrap_or_else(|| {
        // Shift+F3 cycles: lower → UPPER → Title → lower.
        if text.chars().any(char::is_lowercase) && text.chars().any(char::is_uppercase) && !is_title(&text) {
            "lower".into()
        } else if !text.chars().any(char::is_uppercase) {
            "upper".into()
        } else if text.chars().any(char::is_lowercase) {
            "lower".into()
        } else {
            "title".into()
        }
    });
    for path in s.doc.paths_between(&a, &b) {
        let (from, to, src) = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            let from = if path == a.path { a.off } else { 0 };
            let to = if path == b.path { b.off } else { p.len() };
            (from, to, p.text.get(from..to).unwrap_or("").to_string())
        };
        let new = convert(&src, &mode, from == 0);
        if new.len() != src.len() {
            // Lengths differ (ß → SS): replace text keeping the first run's formatting.
            let para = s.doc.para_mut(a.story, &path)?;
            let props = para.props_of_char(from).clone();
            para.delete(from, to)?;
            para.insert_text(from, &new, &props)?;
        } else {
            let para = s.doc.para_mut(a.story, &path)?;
            if let Some(slice) = para.text.get(from..to)
                && slice.len() == new.len()
            {
                para.text.replace_range(from..to, &new);
                para.touch();
            }
        }
    }
    sel_result(s)
}

fn is_title(t: &str) -> bool {
    t.split_whitespace().all(|w| w.chars().next().is_some_and(char::is_uppercase))
}

fn convert(src: &str, mode: &str, para_start: bool) -> String {
    match mode {
        "upper" => src.to_uppercase(),
        "lower" => src.to_lowercase(),
        "toggle" => src.chars().map(|c| if c.is_uppercase() { c.to_lowercase().collect::<String>() } else { c.to_uppercase().collect() }).collect(),
        "title" => {
            let mut out = String::with_capacity(src.len());
            let mut up = true;
            for c in src.chars() {
                if up && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                    up = false;
                } else {
                    out.extend(c.to_lowercase());
                }
                if c.is_whitespace() {
                    up = true;
                }
            }
            out
        }
        _ => {
            // Sentence case.
            let mut out = String::with_capacity(src.len());
            let mut up = para_start;
            for c in src.chars() {
                if up && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                    up = false;
                } else {
                    out.extend(c.to_lowercase());
                }
                if matches!(c, '.' | '!' | '?') {
                    up = true;
                }
            }
            out
        }
    }
}

fn clear(s: &mut Session, _: &Value) -> CmdResult {
    match target(s) {
        Some((a, b)) => {
            s.doc.format_range(&a, &b, &|c| *c = c.cleared())?;
            // Clear Formatting also resets the paragraph to Normal.
            s.doc.format_paragraphs(&a, &b, &|p| {
                let num = p.numbering;
                *p = Default::default();
                p.numbering = num.filter(|_| false);
            })?;
        }
        None => {
            s.pending = Some(CharProps::default());
            let f = s.sel.focus.clone();
            s.doc.format_paragraphs(&f, &f, &|p| *p = Default::default())?;
        }
    }
    sel_result(s)
}

fn char_style(s: &mut Session, v: &Value) -> CmdResult {
    let st = p::req_str(v, "style")?;
    let id = s.doc.styles.find(st).map(|x| x.id.clone()).ok_or_else(|| CmdError::Params(format!("no style `{st}`")))?;
    apply(s, &|c| c.style = Some(id.clone()))
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let props: CharProps = serde_json::from_value(v.get("props").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
    apply(s, &|c| c.overlay(&props))
}

/// What the ribbon shows: the formatting at the selection (mixed values are null).
pub fn state(s: &Session) -> Value {
    let all = resolved(s);
    let first = all.first().cloned();
    let same = |f: &dyn Fn(&ResolvedChar) -> Value| -> Value {
        let Some(r) = &first else { return Value::Null };
        let v0 = f(r);
        if all.iter().all(|r| f(r) == v0) { v0 } else { Value::Null }
    };
    let style = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.style.clone()).unwrap_or_else(|| "Normal".into());
    let style_name = s.doc.styles.get(&style).map(|st| st.name.clone()).unwrap_or(style.clone());
    json!({
        "font": same(&|r| json!(r.font)),
        "size": same(&|r| json!(r.size)),
        "bold": all.iter().all(|r| r.bold),
        "italic": all.iter().all(|r| r.italic),
        "underline": all.iter().all(|r| r.underline != Underline::None),
        "strike": all.iter().all(|r| r.strike),
        "border": all.iter().all(|r| r.border.is_some()),
        "subscript": all.iter().all(|r| r.vert_align == VertAlign::Subscript),
        "superscript": all.iter().all(|r| r.vert_align == VertAlign::Superscript),
        "color": same(&|r| json!(match r.color { TextColor::Auto => "auto".to_string(), TextColor::Rgb(c) => c.hex() })),
        "highlight": same(&|r| json!(r.highlight.map(|c| c.hex()))),
        "style": style,
        "styleName": style_name,
        // The proofing language (a BCP 47 tag; null when mixed or unset) and "Do not check".
        "lang": same(&|r| json!(r.lang)),
        "noProof": all.iter().all(|r| r.no_proof),
    })
}

#[cfg(test)]
mod tests {
    use super::convert;

    #[test]
    fn case_conversions() {
        assert_eq!(convert("hello world. again", "sentence", true), "Hello world. Again");
        assert_eq!(convert("hello WORLD", "title", true), "Hello World");
        assert_eq!(convert("Hello", "toggle", true), "hELLO");
        assert_eq!(convert("straße", "upper", true), "STRASSE");
    }
}
