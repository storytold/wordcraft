//! Character formatting (Home › Font).

use serde_json::{Value, json};
use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
use wordcraft_doc::props::{Border, CharProps, Highlight, Rgb, TextColor, Underline, VertAlign};
use wordcraft_doc::resolve::ResolvedChar;
use wordcraft_doc::{InlineObject, PartKind, Pos, RevisionKind, StoryRef};

use super::sel_result;
use super::tools::{is_turkic, lower_char, upper_char};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Word's font size list.
pub const SIZES: [f32; 17] = [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];

/// Register character-formatting commands, their parameters and keyboard bindings.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("format.bold", "Bold", "Home › Font", |s, v| {
            // The button formats both plain and complex-script text explicitly.
            toggle(
                s,
                v,
                |r| r.bold,
                |c, on| {
                    c.bold = Some(on);
                    c.bold_cs = Some(on);
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
                    c.italic_cs = Some(on);
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
            .key("Mod+Shift+X")
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
        CommandSpec::new("format.font", "Font", "Home › Font", font).key("Mod+Shift+F").params(r#"{"name"?: string, "eastAsia"?: string}"#),
        CommandSpec::new("format.size", "Font Size", "Home › Font", size).key("Mod+Shift+P").params(r#"{"size": number}"#),
        CommandSpec::new("format.growFont", "Increase Font Size", "Home › Font", |s, _| step_size(s, 1)).key("Mod+Shift+. / Mod+Shift+>"),
        CommandSpec::new("format.shrinkFont", "Decrease Font Size", "Home › Font", |s, _| step_size(s, -1)).key("Mod+Shift+, / Mod+Shift+<"),
        CommandSpec::new("format.growFont1", "Grow Font 1 Point", "Home › Font", |s, _| nudge_size(s, 1.0)).key("Mod+]"),
        CommandSpec::new("format.shrinkFont1", "Shrink Font 1 Point", "Home › Font", |s, _| nudge_size(s, -1.0)).key("Mod+["),
        CommandSpec::new("format.color", "Font Color", "Home › Font", color).params(r#"{"color": "RRGGBB" | "auto"}"#),
        CommandSpec::new("format.underlineColor", "Underline Color", "Home › Font › Underline", underline_color)
            .params(r#"{"color": "RRGGBB" | "auto"} (the underline's own colour; "auto" follows the text colour)"#),
        CommandSpec::new("format.highlight", "Text Highlight Color", "Home › Font", highlight)
            .params(
                r#"{"color"?: "yellow|brightGreen|turquoise|pink|blue|red|darkBlue|teal|green|violet|darkRed|darkYellow|gray50|gray25|black|none"} (explicit colour sets it; omitted toggles the last chosen colour, initially yellow)"#,
            )
            // ⌘⌥H hides other apps on macOS.
            .key(super::non_mac("Mod+Alt+H")),
        CommandSpec::new("format.shading", "Character Shading", "Home › Font", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            apply(s, &|x| x.shading = c)
        })
        .params(r#"{"color": "RRGGBB" | null}"#),
        CommandSpec::new("format.changeCase", "Change Case", "Home › Font", change_case)
            .key("Shift+F3")
            .params(r#"{"mode"?: "sentence|lower|upper|title|toggle"}"#),
        CommandSpec::new("format.clear", "Clear All Formatting", "Home › Font", clear),
        // Word's Ctrl+Space; ⌘Space is Spotlight, so Word for Mac uses ⌃Space.
        CommandSpec::new("format.resetChar", "Reset Character Formatting", "Home › Font", reset_char).key(super::mac_or("Ctrl+Space", "Mod+Space")),
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
            s.push_ui_request(json!({"open": "font"}));
            sel_result(s)
        })
        .key("Mod+D")
        .pure(),
        CommandSpec::new("format.enclose", "Enclose Characters", "Home › Font", enclose)
            .params(r#"{"shape": "circle|square|triangle|diamond" | null, "style"?: "shrink|enlarge", "text"?: string}"#),
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

/// A text box selected as an object (its frame, not text in it): all its text, which Word's Font
/// group formats then, instead of the box's anchor in the text around it. A text box filled with
/// a picture (read as a group of its layers, `picture_fill`) is one text box, as in Word.
fn selected_text_box(s: &Session) -> Option<(Pos, Pos)> {
    let id = match super::objects::object_selection(s)?.1 {
        InlineObject::Shape { story: Some(id), .. } => *id,
        o @ InlineObject::Group { picture_fill: true, .. } => o.text_boxes().first().copied()?,
        _ => return None,
    };
    let story = StoryRef::Part(id);
    s.doc.parts.get(&id).filter(|p| p.kind == PartKind::TextBox).map(|_| (s.doc.start_of(story), s.doc.end_of(story)))
}

/// The non-empty rows of a column selection, if one is active. Of whole cells, every paragraph:
/// an empty one's mark takes the formatting.
fn column_rows(s: &Session) -> Option<Vec<(Pos, Pos)>> {
    let segs = s.column_segments()?;
    let cells = s.cell_range().is_some();
    let rows: Vec<(Pos, Pos)> = segs
        .iter()
        .map(|(a, b)| (s.doc.clamp(a), s.doc.clamp(b)))
        .filter(|(a, b)| (a != b || cells) && a.path == b.path && a.story == b.story)
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
            // An empty cell's paragraph only had its mark formatted: lay it out again.
            if a == b {
                s.doc.para_mut(a.story, &a.path)?.touch();
            }
        }
        return sel_result(s);
    }
    match selected_text_box(s).or_else(|| target(s)) {
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

/// Resolved target formatting, sampled up to a paragraph/column-row boundary after 10,000 runs.
/// Empty targets use pending caret formatting or the selected text box paragraph mark.
fn resolved(s: &Session) -> Vec<ResolvedChar> {
    let mut out = Vec::new();
    if let Some(rows) = column_rows(s) {
        for (a, b) in rows {
            let Some(p) = s.doc.para_at(&a) else { continue };
            // An empty cell: its mark's formatting.
            if a == b {
                out.push(s.doc.styles.resolve_char(p.props.style.as_deref(), &p.mark));
            }
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
    let Some((a, b)) = selected_text_box(s).or_else(|| target(s)) else {
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
        let p = s.doc.para_at(&a);
        let style = p.and_then(|p| p.props.style.clone());
        // An empty selected text box has only its paragraph mark, which is what gets formatted.
        let empty_box = selected_text_box(s).and(p).map(|p| &p.mark);
        out.push(s.doc.styles.resolve_char(style.as_deref(), empty_box.unwrap_or(&s.typing_props())));
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

/// `format.font`: `name` is the font (Latin text, and Persian/Arabic text too); `eastAsia` sets the
/// East Asian text's font on its own, as the Font dialog's Asian text font box does (#505).
fn font(s: &mut Session, v: &Value) -> CmdResult {
    let arg = |k: &str| -> Result<Option<String>, CmdError> {
        let Some(n) = v.get(k) else { return Ok(None) };
        let n = n.as_str().map(str::trim).unwrap_or("");
        if n.is_empty() || n.len() > 128 {
            return Err(CmdError::Params(format!("bad font name `{k}`")));
        }
        Ok(Some(n.to_string()))
    };
    let (name, east_asia) = (arg("name")?, arg("eastAsia")?);
    if name.is_none() && east_asia.is_none() {
        return Err(CmdError::Params("`name` or `eastAsia` (string) is required".into()));
    }
    // The font applies to Persian/Arabic text in the selection too (its complex-script font). A
    // Chinese, Japanese or Korean font is the East Asian text's font as well (`w:eastAsia`), as in
    // Word; choosing a Latin font leaves that text's font as it was.
    let ea = east_asia.or_else(|| {
        name.clone().filter(|n| {
            let r = wordcraft_fonts::word::resolve(n, false, false);
            !r.substituted && r.face.is_east_asian()
        })
    });
    apply(s, &|c| {
        if let Some(n) = &name {
            c.font = Some(n.clone());
            c.font_cs = Some(n.clone());
        }
        if let Some(n) = &ea {
            c.font_ea = Some(n.clone());
        }
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
        c.size_cs = Some(sz);
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
        c.size_cs = Some(next);
    })
}

fn nudge_size(s: &mut Session, d: f32) -> CmdResult {
    let next = (current_size(s) + d).clamp(1.0, 1638.0);
    apply(s, &|c| {
        c.size = Some(next);
        c.size_cs = Some(next);
    })
}

fn color(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::req_str(v, "color")?;
    let tc = TextColor::parse(c).ok_or_else(|| CmdError::Params("color must be RRGGBB or auto".into()))?;
    apply(s, &|x| x.color = Some(tc))
}

/// `format.underlineColor`: the underline's own colour (`w:u w:color`), or `auto` to drop it so the
/// underline follows the text colour again. The underline style is left as it is.
fn underline_color(s: &mut Session, v: &Value) -> CmdResult {
    let c = p::req_str(v, "color")?;
    let rgb = match TextColor::parse(c).ok_or_else(|| CmdError::Params("color must be RRGGBB or auto".into()))? {
        TextColor::Auto => None,
        TextColor::Rgb(rgb) => Some(rgb),
    };
    apply(s, &|x| x.underline_color = rgb)
}

/// Test the entire highlight target without the formatting-state query's run cap.
/// Resolve one run at a time and stop at the first mismatch, without collecting them.
fn target_has_highlight(s: &Session, h: Highlight) -> bool {
    let matches = |style: Option<&str>, props: &CharProps| s.doc.styles.resolve_char(style, props).highlight == h.rgb();
    let mut seen = false;
    if let Some(rows) = column_rows(s) {
        for (a, b) in rows {
            let Some(p) = s.doc.para_at(&a) else { continue };
            if a == b || (a.off == 0 && b.off == p.len()) {
                seen = true;
                if !matches(p.props.style.as_deref(), &p.mark) {
                    return false;
                }
            }
            for (r, c) in p.run_ranges() {
                if r.end > a.off && r.start < b.off {
                    seen = true;
                    if !matches(p.props.style.as_deref(), c) {
                        return false;
                    }
                }
            }
        }
        if seen {
            return true;
        }
    }
    if let Some((a, b)) = selected_text_box(s).or_else(|| target(s)) {
        for path in s.doc.paths_between(&a, &b) {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            let from = if path == a.path { a.off } else { 0 };
            let to = if path == b.path { b.off } else { p.len() };
            // Match format_range: fully selected paragraphs include their mark.
            if to == p.len() && (from == 0 || path != a.path) {
                seen = true;
                if !matches(p.props.style.as_deref(), &p.mark) {
                    return false;
                }
            }
            for (r, c) in p.run_ranges() {
                if r.end > from && r.start < to {
                    seen = true;
                    if !matches(p.props.style.as_deref(), c) {
                        return false;
                    }
                }
            }
        }
    }
    // No runs: retain the caret/pending and empty text-box paragraph-mark rules.
    seen || resolved(s).iter().all(|r| r.highlight == h.rgb())
}

/// Explicit palette/automation colours set highlighting; the shortcut (no colour) toggles
/// the last choice off only when the whole formatting target already has that colour.
fn highlight(s: &mut Session, v: &Value) -> CmdResult {
    let color = v.get("color").map(|_| p::req_str(v, "color")).transpose()?;
    let h = match color {
        None => s.last_highlight,
        Some(c) => Highlight::ALL
            .iter()
            .copied()
            .find(|h| {
                h.ooxml().eq_ignore_ascii_case(c)
                    || h.name().replace(['-', ' ', '%'], "").eq_ignore_ascii_case(&c.replace(['-', ' ', '%'], ""))
                    || format!("{h:?}").eq_ignore_ascii_case(c)
            })
            .ok_or_else(|| CmdError::Params(format!("unknown highlight colour `{c}`")))?,
    };
    let remove = color.is_none() && target_has_highlight(s, h);
    let value = if remove { Highlight::None } else { h };
    let result = apply(s, &|x| x.highlight = Some(value))?;
    s.last_highlight = h;
    Ok(result)
}

/// `format.changeCase`: `mode` (lower, upper, title, sentence, toggle), or Shift+F3's cycle
/// without one. Works on the selection, the word at the caret, or a selected text box's text.
fn change_case(s: &mut Session, v: &Value) -> CmdResult {
    let (a, b) = match selected_text_box(s) {
        Some(all) => all,
        None => {
            let (a, b) = s.sel.ordered();
            if a == b { target(s).unwrap_or((a, b)) } else { (a, b) }
        }
    };
    if a == b {
        return sel_result(s);
    }
    // Changing case rewrites the characters: refused in content locked against editing.
    super::controls::check_edit(s, &a, &b)?;
    // Only text that isn't a tracked deletion counts, for the mode and the conversion: after a
    // tracked change of case the old letters are still there, deleted (#1329).
    let text: String = s
        .doc
        .paths_between(&a, &b)
        .iter()
        .filter_map(|path| {
            let p = s.doc.para(a.story, path)?;
            let from = if *path == a.path { a.off } else { 0 };
            let to = if *path == b.path { b.off } else { p.len() };
            Some(live_chars(p, from, to).into_iter().map(|(_, c)| c).collect::<String>())
        })
        .collect::<Vec<_>>()
        .join("\n");
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
        // Changed stretches of one run each: (start, end, replacement), in text order.
        let groups = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            let from = if path == a.path { a.off } else { 0 };
            let to = if path == b.path { b.off } else { p.len() };
            let chars = live_chars(p, from, to);
            let src: String = chars.iter().map(|(_, c)| c).collect();
            // Turkish, Azerbaijani and Crimean Tatar text follows the same case mapping as
            // AutoCorrect (#1923); each run's own language decides (#3479).
            let turkic: Vec<bool> = chars
                .iter()
                .map(|(off, _)| {
                    let lang = s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(*off)).lang;
                    is_turkic(lang.as_deref().or(s.editing_lang.as_deref()))
                })
                .collect();
            case_groups(p, &chars, &convert_chars(&src, &mode, from == 0, &turkic))
        };
        if groups.is_empty() {
            continue;
        }
        // With Track Changes on, a changed stretch is a tracked deletion of the old characters
        // followed by a tracked insertion of the new, so Reject All restores the old case (#1329).
        // Text this author inserted is rewritten in place (it stays one insertion); text already
        // deleted isn't in the stretches.
        let tracked = s.doc.settings.track_changes;
        let kinds: Vec<Tracked> = {
            let Some(p) = s.doc.para(a.story, &path) else { continue };
            groups
                .iter()
                .map(|(start, _, _)| {
                    let c = p.props_of_char(*start);
                    let own = c.ins.and_then(|i| s.doc.revisions.get(i as usize)).is_some_and(|rv| rv.author == s.author);
                    if tracked && !own { Tracked::Replace } else { Tracked::No }
                })
                .collect()
        };
        let revs = if kinds.contains(&Tracked::Replace) {
            Some((super::new_revision(s, RevisionKind::Delete), super::new_revision(s, RevisionKind::Insert)))
        } else {
            None
        };
        // Each stretch keeps its own run's formatting (#421); right to left so offsets hold.
        let para = s.doc.para_mut(a.story, &path)?;
        let mut spans: Vec<(usize, usize, usize)> = Vec::with_capacity(groups.len());
        for ((start, end, new), kind) in groups.iter().zip(&kinds).rev() {
            let old_len = end.saturating_sub(*start);
            match (kind, revs) {
                (Tracked::Replace, Some((del, ins))) => {
                    let mut props = para.props_of_char(*start).clone();
                    para.format(*start, *end, &|c| c.del = Some(del))?;
                    (props.ins, props.del, props.fmt_change) = (Some(ins), None, None);
                    para.insert_text(*end, new, &props)?;
                    spans.push((*start, *end, old_len.saturating_add(new.len())));
                }
                _ => {
                    if new.len() == old_len {
                        if para.text.get(*start..*end).is_some() {
                            para.text.replace_range(*start..*end, new);
                        }
                    } else {
                        let props = para.props_of_char(*start).clone();
                        para.delete(*start, *end)?;
                        para.insert_text(*start, new, &props)?;
                    }
                    spans.push((*start, *end, new.len()));
                }
            }
        }
        spans.reverse();
        para.touch();
        // The selection keeps covering the converted text when its length changed (#420).
        for end in [&mut s.sel.anchor, &mut s.sel.focus] {
            if end.story == a.story && end.path == path {
                end.off = map_offset(&spans, end.off);
            }
        }
    }
    sel_result(s)
}

/// How Change Case rewrites one stretch under Track Changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tracked {
    /// In place: tracking is off, or the text is this author's own insertion.
    No,
    /// A tracked deletion of the old text and a tracked insertion of the new.
    Replace,
}

/// The characters of `p` in `from..to` that aren't tracked deletions, with their offsets.
fn live_chars(p: &wordcraft_doc::Paragraph, from: usize, to: usize) -> Vec<(usize, char)> {
    let mut out = Vec::new();
    for (r, c) in p.run_ranges() {
        let (x, y) = (r.start.max(from), r.end.min(to));
        if c.del.is_some() || x >= y {
            continue;
        }
        if let Some(t) = p.text.get(x..y) {
            out.extend(t.char_indices().map(|(i, ch)| (x + i, ch)));
        }
    }
    out
}

/// Group the characters `chars` (offsets in `p`) whose converted form `conv` differs into
/// stretches of adjacent characters that stay inside one run: `(start, end, replacement)`, in
/// text order.
fn case_groups(p: &wordcraft_doc::Paragraph, chars: &[(usize, char)], conv: &[String]) -> Vec<(usize, usize, String)> {
    let runs: Vec<std::ops::Range<usize>> = p.run_ranges().map(|(r, _)| r).collect();
    let run_of = |off: usize| runs.iter().position(|r| r.contains(&off));
    let mut out: Vec<(usize, usize, String, Option<usize>)> = Vec::new();
    for (&(start, c), new) in chars.iter().zip(conv) {
        let end = start + c.len_utf8();
        let mut buf = [0u8; 4];
        if new.as_str() == c.encode_utf8(&mut buf) {
            continue;
        }
        let run = run_of(start);
        match out.last_mut() {
            Some(g) if g.1 == start && g.3 == run => {
                g.1 = end;
                g.2.push_str(new);
            }
            _ => out.push((start, end, new.clone(), run)),
        }
    }
    out.into_iter().map(|(a, b, n, _)| (a, b, n)).collect()
}

/// Where `off` lands after the stretches `(start, end, new_len)` (text order) were replaced;
/// an offset inside a stretch moves to its end.
fn map_offset(spans: &[(usize, usize, usize)], off: usize) -> usize {
    let mut out = off;
    for &(start, end, new_len) in spans {
        if end <= off {
            out = out.saturating_sub(end.saturating_sub(start)).saturating_add(new_len);
        } else if start < off {
            out = out.saturating_sub(off - start).saturating_add(new_len);
        }
    }
    out
}

fn is_title(t: &str) -> bool {
    t.split_whitespace().all(|w| w.chars().next().is_some_and(char::is_uppercase))
}

#[cfg(test)]
fn convert(src: &str, mode: &str, para_start: bool) -> String {
    let turkic = vec![false; src.chars().count()];
    convert_chars(src, mode, para_start, &turkic).concat()
}

fn convert_title_char(c: char, i: usize, up: &mut bool, upper: impl Fn(char) -> String, lower: impl Fn(usize, char) -> String) -> String {
    let n = if *up && c.is_alphabetic() {
        *up = false;
        upper(c)
    } else {
        lower(i, c)
    };
    if c.is_whitespace() {
        *up = true;
    }
    n
}

fn convert_sentence_char(
    c: char,
    i: usize,
    up: &mut bool,
    chars: &[char],
    upper: impl Fn(char) -> String,
    lower: impl Fn(usize, char) -> String,
) -> String {
    let n = if *up && c.is_alphabetic() {
        *up = false;
        upper(c)
    } else {
        lower(i, c)
    };
    let is_decimal = c == '.'
        && i.checked_sub(1).and_then(|j| chars.get(j)).is_some_and(char::is_ascii_digit)
        && chars.get(i + 1).is_some_and(char::is_ascii_digit);
    if matches!(c, '!' | '?') || (c == '.' && !is_decimal) {
        *up = true;
    }
    n
}

/// The case-converted form of each character of `src`, in order (one entry per char, so a
/// change of length such as ß → SS or İ → i̇ can be applied run by run). `turkic` is one flag
/// per char: true when that character's run is Turkish, Azerbaijani or Crimean Tatar.
fn convert_chars(src: &str, mode: &str, para_start: bool, turkic: &[bool]) -> Vec<String> {
    let chars: Vec<char> = src.chars().collect();
    // Greek capital sigma lowers to the final form at the end of a word (as `str::to_lowercase`).
    let lower = |i: usize, c: char| -> String {
        let prev = i.checked_sub(1).and_then(|j| chars.get(j)).is_some_and(|p| p.is_alphabetic());
        let next = chars.get(i + 1).is_some_and(|n| n.is_alphabetic());
        if c == 'Σ' && prev && !next {
            "ς".to_string()
        } else if turkic.get(i).copied().unwrap_or(false) {
            lower_char(c, true)
        } else {
            c.to_lowercase().collect()
        }
    };
    // Title case capitalises the first word of any selection; sentence case only at a paragraph start.
    let mut up = mode == "title" || para_start;
    let mut out = Vec::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        let turkic = turkic.get(i).copied().unwrap_or(false);
        let upper = |c: char| upper_char(c, turkic);
        let new = match mode {
            "upper" => upper(c),
            "lower" => lower(i, c),
            "toggle" => {
                if c.is_uppercase() {
                    lower(i, c)
                } else {
                    upper(c)
                }
            }
            "title" => convert_title_char(c, i, &mut up, upper, lower),
            _ => convert_sentence_char(c, i, &mut up, &chars, upper, lower),
        };
        out.push(new);
    }
    out
}

/// `format.clear` (Clear All Formatting): direct character formatting off and the paragraphs back
/// to Normal, for the selection or a selected text box's text; at a caret, for what's typed next.
fn clear(s: &mut Session, _: &Value) -> CmdResult {
    match selected_text_box(s).or_else(|| target(s)) {
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

/// Remove direct character formatting, keeping the paragraph and any character style.
fn reset_char(s: &mut Session, _: &Value) -> CmdResult {
    match selected_text_box(s).or_else(|| target(s)) {
        Some((a, b)) => s.doc.format_range(&a, &b, &|c| *c = CharProps { style: c.style.clone(), ..c.cleared() })?,
        None => s.pending = Some(CharProps::default()),
    }
    sel_result(s)
}

fn char_style(s: &mut Session, v: &Value) -> CmdResult {
    let st = p::req_str(v, "style")?;
    let id = s.doc.styles.find(st).map(|x| x.id.clone()).ok_or_else(|| CmdError::Params(format!("no style `{st}`")))?;
    apply(s, &|c| c.style = Some(id.clone()))
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let mut props: CharProps = serde_json::from_value(v.get("props").cloned().unwrap_or(Value::Null)).map_err(|e| CmdError::Params(e.to_string()))?;
    // The font dialog and callers supplying one value format both scripts. Explicit
    // complex-script values still take precedence; importing a DOCX never uses this command.
    props.size_cs = props.size_cs.or(props.size);
    props.bold_cs = props.bold_cs.or(props.bold);
    props.italic_cs = props.italic_cs.or(props.italic);
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
    // A selected text box shows its own (first) paragraph's style, like its text's formatting.
    let at = selected_text_box(s).map_or_else(|| s.sel.focus.clone(), |(a, _)| a);
    let style = s.doc.para_at(&at).and_then(|p| p.props.style.clone()).unwrap_or_else(|| "Normal".into());
    let style_name = s.doc.styles.get(&style).map(|st| st.name.clone()).unwrap_or(style.clone());
    json!({
        "font": same(&|r| json!(r.font)),
        "fontEastAsia": same(&|r| json!(r.font_ea)),
        "size": same(&|r| json!(r.size)),
        "bold": all.iter().all(|r| r.bold),
        "italic": all.iter().all(|r| r.italic),
        "underline": all.iter().all(|r| r.underline != Underline::None),
        "underlineColor": same(&|r| json!(r.underline_color.map_or_else(|| "auto".to_string(), |c| c.hex()))),
        "strike": all.iter().all(|r| r.strike),
        "border": all.iter().all(|r| r.border.is_some()),
        "subscript": all.iter().all(|r| r.vert_align == VertAlign::Subscript),
        "superscript": all.iter().all(|r| r.vert_align == VertAlign::Superscript),
        "smallCaps": all.iter().all(|r| r.small_caps),
        "caps": all.iter().all(|r| r.caps),
        "hidden": all.iter().all(|r| r.hidden),
        "spacing": same(&|r| json!(r.spacing)),
        // Kerning for fonts: the smallest kerned size, points; 0 = off.
        "kern": same(&|r| json!(r.kern)),
        "color": same(&|r| json!(match r.color { TextColor::Auto => "auto".to_string(), TextColor::Rgb(c) => c.hex() })),
        "highlight": same(&|r| json!(r.highlight.map(|c| c.hex()))),
        "style": style,
        "styleName": style_name,
        // The proofing language (a BCP 47 tag; null when mixed or unset) and "Do not check".
        "lang": same(&|r| json!(r.lang)),
        "noProof": all.iter().all(|r| r.no_proof),
    })
}

/// What Enclose Characters works on: the selected text (one full-width or up to two half-width
/// characters), or enclosed characters already there (selected, or beside the caret).
#[derive(Clone, Debug, PartialEq)]
pub struct EncloseTarget {
    /// The range to replace.
    pub from: Pos,
    pub to: Pos,
    pub text: String,
    pub props: CharProps,
    /// The enclosure there now.
    pub current: Option<Enclosure>,
}

/// The selection as Enclose Characters sees it (an error says why it can't be enclosed).
pub fn enclose_target(s: &Session) -> Result<EncloseTarget, CmdError> {
    use wordcraft_doc::para::OBJ;
    let (a, b) = s.sel.ordered();
    if a.story != b.story || a.path != b.path {
        return Err(CmdError::Failed("select characters within one paragraph".into()));
    }
    let para = s.doc.para_at(&a).ok_or_else(|| CmdError::Failed("no paragraph at the selection".into()))?;
    let enclosed = |off: usize| -> Option<EncloseTarget> {
        let Some(wordcraft_doc::InlineObject::Field { instr, .. }) = para.object_at(off) else { return None };
        let e = Enclosure::parse(instr)?;
        let end = off + OBJ.len_utf8();
        Some(EncloseTarget {
            from: Pos { off, ..a.clone() },
            to: Pos { off: end, ..a.clone() },
            text: e.text.clone(),
            props: para.props_at(end).clone(),
            current: Some(e),
        })
    };
    if a.off == b.off {
        // A caret just after (or before) enclosed characters.
        let before = para.text.get(..a.off).and_then(|t| t.chars().next_back()).filter(|c| *c == OBJ).map(|c| a.off - c.len_utf8());
        return before.and_then(enclosed).or_else(|| enclosed(a.off)).ok_or_else(|| CmdError::Failed("select the characters to enclose".into()));
    }
    let text = para.text.get(a.off..b.off).ok_or_else(|| CmdError::Failed("bad selection".into()))?;
    if text == OBJ.to_string()
        && let Some(t) = enclosed(a.off)
    {
        return Ok(t);
    }
    if !wordcraft_doc::eq::fits_enclosure(text) {
        return Err(CmdError::Failed("select one full-width character or up to two half-width characters".into()));
    }
    let first = text.chars().next().map(char::len_utf8).unwrap_or(0);
    Ok(EncloseTarget { from: a.clone(), to: b, text: text.to_string(), props: para.props_at(a.off + first).clone(), current: None })
}

/// Home › Font › Enclose Characters: put the selected character(s) in a circle, square, triangle
/// or diamond (an `EQ \o` field, see [`wordcraft_doc::eq`]), or (`shape: null`) take them out.
fn enclose(s: &mut Session, v: &Value) -> CmdResult {
    let shape = match v.get("shape") {
        None => return Err(CmdError::Params("`shape` is required: circle, square, triangle, diamond, or null to remove".into())),
        Some(Value::Null) => None,
        Some(x) => {
            Some(x.as_str().and_then(EncloseShape::parse).ok_or_else(|| CmdError::Params("shape: circle|square|triangle|diamond|null".into()))?)
        }
    };
    let style = match p::str(v, "style") {
        Some(x) => Some(EncloseStyle::parse(x).ok_or_else(|| CmdError::Params("style: shrink|enlarge".into()))?),
        None => None,
    };
    // Text typed in the dialog replaces the selection's (or goes in at the caret).
    let given = match p::str(v, "text") {
        Some(t) if shape.is_some() && !wordcraft_doc::eq::fits_enclosure(t) => {
            return Err(CmdError::Params("text: one full-width character or up to two half-width characters".into()));
        }
        Some(t) if shape.is_some() => Some(t.to_string()),
        _ => None,
    };
    let t = match (enclose_target(s), given) {
        (Ok(t), None) => t,
        (Ok(t), Some(text)) => EncloseTarget { text, ..t },
        (Err(_), Some(text)) if s.sel.anchor == s.sel.focus => {
            let at = s.sel.focus.clone();
            EncloseTarget { from: at.clone(), to: at, text, props: s.typing_props(), current: None }
        }
        (Err(e), _) => return Err(e),
    };
    if shape.is_none() && t.current.is_none() {
        return Err(CmdError::Failed("no enclosed characters at the selection".into()));
    }
    s.sel = crate::Selection { anchor: t.from.clone(), focus: t.to.clone() };
    let at = super::delete_selection(s)?;
    let end = match shape {
        Some(shape) => {
            let style = style.or(t.current.as_ref().map(|e| e.style)).unwrap_or_default();
            let e = Enclosure { shape, style, text: t.text.clone() };
            let obj = wordcraft_doc::InlineObject::Field {
                instr: e.instr(),
                result: t.text,
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            };
            s.doc.insert_object(&at, obj, &t.props)?
        }
        None => s.doc.insert_text(&at, &t.text, &t.props)?,
    };
    s.sel = crate::Selection { anchor: at, focus: end };
    sel_result(s)
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
        assert_eq!(convert("hello world", "title", false), "Hello World");
        assert_eq!(convert("ΣΟΦΟΣ ΟΔΟΣ", "lower", true), "σοφος οδος");
    }

    #[test]
    fn sentence_case_ignores_decimal_points() {
        // #3485: A period between two digits is a decimal point, not a sentence boundary.
        assert_eq!(convert("one 3.14 apples and two 2.5 pears.", "sentence", true), "One 3.14 apples and two 2.5 pears.");
        assert_eq!(convert("pi is 3.14. next sentence.", "sentence", true), "Pi is 3.14. Next sentence.");
    }
}
