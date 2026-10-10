//! Design tab: themes, style sets, colours, fonts, paragraph spacing, watermark, page colour, borders.

use serde_json::{Value, json};
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing, Rgb, TextColor};
use wordcraft_doc::{Watermark, styles};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Our own themes: (name, heading font, body font, accent colours).
pub const THEMES: &[(&str, &str, &str, [Rgb; 6])] = &[
    (
        "Craft",
        styles::HEADING_FONT,
        styles::BODY_FONT,
        [Rgb(0x15, 0x60, 0x82), Rgb(0xE9, 0x71, 0x32), Rgb(0x19, 0x6B, 0x24), Rgb(0x0F, 0x9E, 0xD5), Rgb(0xA0, 0x2B, 0x93), Rgb(0x4E, 0xA7, 0x2E)],
    ),
    (
        "Studio",
        "Georgia",
        "Georgia",
        [Rgb(0x8C, 0x2F, 0x39), Rgb(0xC9, 0x8B, 0x2E), Rgb(0x3E, 0x5C, 0x76), Rgb(0x6B, 0x8F, 0x71), Rgb(0x9A, 0x6F, 0xB0), Rgb(0x4A, 0x4A, 0x4A)],
    ),
    (
        "Gallery",
        "Helvetica Neue",
        "Helvetica Neue",
        [Rgb(0x22, 0x22, 0x22), Rgb(0xE6, 0x39, 0x46), Rgb(0x45, 0x7B, 0x9D), Rgb(0x1D, 0x35, 0x57), Rgb(0xA8, 0xDA, 0xDC), Rgb(0xF4, 0xA2, 0x61)],
    ),
    (
        "Atelier",
        "Palatino",
        "Palatino",
        [Rgb(0x6D, 0x59, 0x7A), Rgb(0xB5, 0x65, 0x76), Rgb(0xE5, 0x6B, 0x6F), Rgb(0xEA, 0xAC, 0x8B), Rgb(0x35, 0x50, 0x70), Rgb(0x8E, 0x9A, 0xAF)],
    ),
    (
        "Darkroom",
        "Futura",
        "Avenir Next",
        [Rgb(0x26, 0x46, 0x53), Rgb(0x2A, 0x9D, 0x8F), Rgb(0xE9, 0xC4, 0x6A), Rgb(0xF4, 0xA2, 0x61), Rgb(0xE7, 0x6F, 0x51), Rgb(0x5E, 0x54, 0x8E)],
    ),
    (
        "Sketchbook",
        "Gill Sans",
        "Gill Sans",
        [Rgb(0x58, 0x81, 0x57), Rgb(0x3A, 0x5A, 0x40), Rgb(0xA3, 0xB1, 0x8A), Rgb(0xDA, 0xD7, 0xCD), Rgb(0x34, 0x4E, 0x41), Rgb(0xBC, 0x6C, 0x25)],
    ),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("design.theme", "Themes", "Design › Document Formatting", theme).params(r#"{"name": string}"#),
        CommandSpec::new("design.themeFonts", "Fonts", "Design › Document Formatting", |s, v| {
            let heading = p::req_str(v, "heading")?.to_string();
            let body = p::str(v, "body").unwrap_or(&heading).to_string();
            set_fonts(s, &heading, &body);
            sel_result(s)
        })
        .params(r#"{"heading": string, "body"?: string}"#),
        CommandSpec::new("design.themeColors", "Colors", "Design › Document Formatting", |s, v| {
            let name = p::req_str(v, "name")?;
            let t = THEMES.iter().find(|t| t.0.eq_ignore_ascii_case(name)).ok_or_else(|| CmdError::Params(format!("unknown colours `{name}`")))?;
            set_accent(s, t.3[0]);
            sel_result(s)
        }),
        CommandSpec::new("design.styleSet", "Style Set", "Design › Document Formatting", style_set)
            .params(r#"{"name": "default|basic|lines|shaded|casual|centered|minimalist|title"}"#),
        CommandSpec::new("design.paragraphSpacing", "Paragraph Spacing", "Design › Document Formatting", para_spacing)
            .params(r#"{"value": "default|none|compact|tight|open|relaxed|double"}"#),
        CommandSpec::new("design.watermark", "Watermark", "Design › Page Background", watermark)
            .params(r#"{"text"?: string, "remove"?: bool, "diagonal"?: bool, "color"?: "RRGGBB"}"#),
        CommandSpec::new("design.pageColor", "Page Color", "Design › Page Background", |s, v| {
            s.doc.settings.page_color = p::str(v, "color").and_then(Rgb::parse);
            sel_result(s)
        })
        .params(r#"{"color": "RRGGBB" | null}"#),
        CommandSpec::new("design.pageBorders", "Page Borders", "Design › Page Background", page_borders).params(
            r#"{"kind"?: "box|none", "width"?: pt, "color"?: "RRGGBB", "style"?: "single|double|dotted|dashed|thick|triple|dotDash|wave", "sides"?: {"top"|"left"|"bottom"|"right": {"style"?, "width"?: pt, "color"?: "RRGGBB"} | null} (exactly these sides, instead of `kind`), "applyTo"?: "section|document"}"#,
        ),
        CommandSpec::new("design.setDefault", "Set as Default", "Design › Document Formatting", |s, _| {
            s.status = "These settings will be used for new documents.".into();
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("design.themes", "Theme List", "Design › Document Formatting", |_, _| {
            Ok(json!(THEMES.iter().map(|t| t.0).collect::<Vec<_>>()))
        })
        .pure(),
    ]
}

fn set_fonts(s: &mut Session, heading: &str, body: &str) {
    s.doc.settings.major_font = heading.into();
    s.doc.settings.minor_font = body.into();
    s.doc.styles.default_chr.font = Some(body.into());
    for st in &mut s.doc.styles.styles {
        if let Some(f) = &st.chr.font
            && (f == styles::HEADING_FONT || st.id.starts_with("Heading") || st.id == "Title")
        {
            st.chr.font = Some(heading.into());
        } else if st.chr.font.is_some() && st.chr.font.as_deref() != Some(heading) {
            st.chr.font = Some(body.into());
        }
    }
}

fn set_accent(s: &mut Session, accent: Rgb) {
    for st in &mut s.doc.styles.styles {
        if st.id.starts_with("Heading") && st.id.len() == 8 && st.id.as_bytes().get(7).is_some_and(|d| *d <= b'5')
            || st.id == "IntenseQuote"
            || st.id == "IntenseEmphasis"
            || st.id == "IntenseReference"
        {
            st.chr.color = Some(TextColor::Rgb(accent));
            if let Some(b) = st.para.borders.as_mut() {
                for e in [&mut b.top, &mut b.bottom].into_iter().flatten() {
                    e.color = Some(accent);
                }
            }
        }
    }
    if let Some(c) = s.doc.settings.theme_colors.get_mut(4) {
        *c = accent;
    }
}

fn theme(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?;
    let t = THEMES.iter().find(|t| t.0.eq_ignore_ascii_case(name)).ok_or_else(|| CmdError::Params(format!("unknown theme `{name}`")))?;
    set_fonts(s, t.1, t.2);
    set_accent(s, t.3[0]);
    s.doc.settings.theme_name = t.0.into();
    let mut colors = s.doc.settings.theme_colors.clone();
    for (i, c) in t.3.iter().enumerate() {
        if let Some(x) = colors.get_mut(4 + i) {
            *x = *c;
        }
    }
    s.doc.settings.theme_colors = colors;
    sel_result(s)
}

fn style_set(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?;
    let accent = s.doc.settings.theme_colors.get(4).copied().unwrap_or(styles::HEADING_BLUE);
    let mut fresh = wordcraft_doc::StyleSheet::builtin();
    let line = Border { style: BorderStyle::Single, width: 0.75, color: Some(accent), space: 1.0 };
    for st in &mut fresh.styles {
        let heading = st.id.starts_with("Heading") || st.id == "Title";
        match name {
            "basic" if heading => {
                st.chr.color = Some(TextColor::Rgb(Rgb(0x26, 0x26, 0x26)));
                st.chr.bold = Some(true);
            }
            "lines" if st.id == "Heading1" || st.id == "Title" => st.para.borders = Some(Borders { bottom: Some(line), ..Default::default() }),
            "shaded" if st.id == "Heading1" => {
                st.para.shading = Some(accent);
                st.chr.color = Some(TextColor::Rgb(Rgb::WHITE));
            }
            "casual" if heading => st.chr.italic = Some(true),
            "centered" if heading => st.para.align = Some(wordcraft_doc::Align::Center),
            "minimalist" if heading => {
                st.chr.caps = Some(true);
                st.chr.spacing = Some(1.0);
                st.chr.color = Some(TextColor::Rgb(Rgb(0x40, 0x40, 0x40)));
            }
            "title" if st.id == "Title" => st.chr.size = Some(36.0),
            _ => {}
        }
    }
    // Keep custom styles and fonts.
    let customs: Vec<_> = s.doc.styles.styles.iter().filter(|x| !x.builtin).cloned().collect();
    fresh.styles.extend(customs);
    fresh.default_chr.font = s.doc.styles.default_chr.font.clone();
    s.doc.styles = fresh;
    let (h, b) = (s.doc.settings.major_font.clone(), s.doc.settings.minor_font.clone());
    set_fonts(s, &h, &b);
    set_accent(s, accent);
    sel_result(s)
}

fn para_spacing(s: &mut Session, v: &Value) -> CmdResult {
    let (before, after, line) = match p::req_str(v, "value")? {
        "none" => (0.0, 0.0, 1.0),
        "compact" => (0.0, 4.0, 1.0),
        "tight" => (0.0, 6.0, 1.15),
        "open" => (0.0, 10.0, 1.15),
        "relaxed" => (0.0, 6.0, 1.5),
        "double" => (0.0, 8.0, 2.0),
        _ => (0.0, 8.0, 1.15),
    };
    s.doc.styles.default_para.space_before = Some(before);
    s.doc.styles.default_para.space_after = Some(after);
    s.doc.styles.default_para.line_spacing = Some(LineSpacing::Multiple(line));
    sel_result(s)
}

fn watermark(s: &mut Session, v: &Value) -> CmdResult {
    if p::bool(v, "remove").unwrap_or(false) {
        s.doc.settings.watermark = None;
        return sel_result(s);
    }
    let mut wm = s.doc.settings.watermark.clone().unwrap_or_default();
    if let Some(t) = p::str(v, "text") {
        wm.text = t.chars().take(200).collect();
    }
    if let Some(d) = p::bool(v, "diagonal") {
        wm.diagonal = d;
    }
    if let Some(c) = p::str(v, "color").and_then(Rgb::parse) {
        wm.color = c;
    }
    s.doc.settings.watermark = Some(Watermark { ..wm });
    sel_result(s)
}

/// Page borders of the caret's section (or every section with `applyTo: "document"`).
fn page_borders(s: &mut Session, v: &Value) -> CmdResult {
    let borders = match v.get("sides") {
        Some(sides) => Some(super::para::border_sides(sides, 24.0, 24.0)?).filter(Borders::any_visible),
        None => {
            let kind = p::str(v, "kind").unwrap_or("box");
            let width = p::f32(v, "width").unwrap_or(1.0).clamp(0.25, 6.0);
            let color = p::str(v, "color").and_then(Rgb::parse);
            let style = p::str(v, "style").map(BorderStyle::from_ooxml).unwrap_or(BorderStyle::Single);
            (kind != "none").then(|| Borders::box_(Border { style, width, color, space: 24.0 }))
        }
    };
    let blocks: Vec<usize> = match p::str(v, "applyTo") {
        Some("document") => s.doc.sections().iter().map(|(end, _)| *end).collect(),
        Some("section") | None => vec![s.sel.focus.path.0.first().copied().unwrap_or(0) as usize],
        Some(x) => return Err(CmdError::Params(format!("unknown `applyTo` `{x}`"))),
    };
    for block in blocks {
        s.doc.section_mut(block).page_borders = borders;
    }
    sel_result(s)
}
