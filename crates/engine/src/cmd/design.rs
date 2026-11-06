//! Design tab: themes, style sets, colours, fonts, paragraph spacing, watermark, page colour, borders.

use serde_json::{Value, json};
use std::sync::Arc;
use wordcraft_doc::props::{Border, BorderStyle, Borders, CharProps, LineSpacing, Rgb, TextColor};
use wordcraft_doc::watermark::{MAX_SCALE, MIN_SCALE};
use wordcraft_doc::{WatermarkPicture, styles};

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
        CommandSpec::new("design.scaleFontSizes", "Scale Font Sizes", "Design › Document Formatting", scale_font_sizes)
            .params(r#"{"delta": pt (-1638..1638, not 0) | "percent": number (-99..1000, not 0; 10 = +10%)} (exactly one)"#),
        CommandSpec::new("design.watermark", "Watermark", "Design › Page Background", watermark)
            .params(r#"{"text"?: string (a text watermark), "remove"?: bool, "diagonal"?: bool, "color"?: "RRGGBB", "transparency"?: 0-100 (percent; Word's Semitransparent is 50), "semitransparent"?: bool (50 or 0 percent), "picture"?: {"path": string} | {"data": base64} | {"media": key} (a picture watermark; at most 64 MB) | false (back to text), "scale"?: "auto" | 1-500 (percent of the picture's own size; auto fits the margins), "washout"?: bool (faded, default on)}"#),
        CommandSpec::new("design.pageColor", "Page Color", "Design › Page Background", |s, v| {
            s.doc.settings.page_color = p::str(v, "color").and_then(Rgb::parse);
            s.doc.settings.page_picture = None;
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
    Arc::make_mut(&mut s.doc.styles).default_chr.font = Some(body.into());
    for st in &mut Arc::make_mut(&mut s.doc.styles).styles {
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
    for st in &mut Arc::make_mut(&mut s.doc.styles).styles {
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
    // Overlay the set onto the document's sheet: every other style, the latent styles and the
    // document defaults stay as they are (no set defines its own defaults).
    let sheet = Arc::make_mut(&mut s.doc.styles);
    for mut st in fresh.styles {
        match sheet.styles.iter_mut().find(|x| x.id == st.id) {
            Some(old) => {
                st.default = old.default;
                st.locked = old.locked;
                st.builtin = old.builtin;
                *old = st;
            }
            None => sheet.styles.push(st),
        }
    }
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
    Arc::make_mut(&mut s.doc.styles).default_para.space_before = Some(before);
    Arc::make_mut(&mut s.doc.styles).default_para.space_after = Some(after);
    Arc::make_mut(&mut s.doc.styles).default_para.line_spacing = Some(LineSpacing::Multiple(line));
    sel_result(s)
}

/// Grow or shrink every style's font size at once. Only explicit style sizes change (including
/// table style parts); styles that inherit one (via `based_on` or the defaults) follow, and runs
/// keep their own direct formatting.
fn scale_font_sizes(s: &mut Session, v: &Value) -> CmdResult {
    let (delta, percent) = (p::f32(v, "delta"), p::f32(v, "percent"));
    // A field that is present but not a number is an error, not "absent".
    if (v.get("delta").is_some() && delta.is_none()) || (v.get("percent").is_some() && percent.is_none()) {
        return Err(CmdError::Params("`delta` and `percent` must be numbers".into()));
    }
    if delta.is_some() == percent.is_some() {
        return Err(CmdError::Params("give exactly one of `delta` (points) or `percent`".into()));
    }
    if delta.is_some_and(|d| !(-1638.0..=1638.0).contains(&d) || d == 0.0) || percent.is_some_and(|x| !(-99.0..=1000.0).contains(&x) || x == 0.0) {
        return Err(CmdError::Params("`delta` must be non-zero within ±1638 pt and `percent` non-zero within -99..1000".into()));
    }
    let factor = 1.0 + percent.unwrap_or(0.0) / 100.0;
    let scale = |size: f32| ((size * factor + delta.unwrap_or(0.0)).clamp(1.0, 1638.0) * 2.0).round() / 2.0;
    let scale_chr = |c: &mut CharProps| {
        c.size = c.size.map(scale);
        c.size_cs = c.size_cs.map(scale);
    };
    let st = Arc::make_mut(&mut s.doc.styles);
    // An unset document default resolves to the fallback size; make it explicit so inheriting styles follow.
    st.default_chr.size = Some(scale(st.default_chr.size.unwrap_or(wordcraft_doc::props::DEFAULT_FONT_SIZE)));
    st.default_chr.size_cs = st.default_chr.size_cs.map(scale);
    for style in &mut st.styles {
        scale_chr(&mut style.chr);
        if let Some(t) = style.table.as_mut() {
            for c in [&mut t.header_chr, &mut t.band_chr, &mut t.first_col_chr, &mut t.last_col_chr, &mut t.total_chr, &mut t.col_band_chr] {
                scale_chr(c);
            }
            for part in t.regions.values_mut() {
                scale_chr(&mut part.chr);
            }
        }
    }
    sel_result(s)
}

/// Design › Watermark: a text watermark (`text` switches to one), or a picture watermark
/// (`picture`) with its scale and Washout; the text keeps its settings meanwhile, as in Word's
/// dialog. Settings for the other kind are refused rather than silently kept.
fn watermark(s: &mut Session, v: &Value) -> CmdResult {
    if p::bool(v, "remove").unwrap_or(false) {
        s.doc.settings.watermark = None;
        return sel_result(s);
    }
    let mut wm = s.doc.settings.watermark.clone().unwrap_or_default();
    // Everything is checked before the document is touched (a picture added to the media).
    let scale = match v.get("scale") {
        None => None,
        Some(Value::String(a)) if a.eq_ignore_ascii_case("auto") => Some(None),
        Some(_) => {
            let percent =
                p::f32(v, "scale").filter(|p| p.is_finite()).ok_or_else(|| CmdError::Params("`scale` is \"auto\" or a percentage".into()))?;
            // Tenths of a percent, as a saved file gives it back.
            Some(Some((percent.clamp(MIN_SCALE, MAX_SCALE) * 10.0).round() / 10.0))
        }
    };
    let picked = match v.get("picture") {
        Some(Value::Bool(false)) => {
            wm.picture = None;
            None
        }
        Some(pic) if !pic.is_null() => Some(watermark_picture(s, pic)?),
        _ => {
            if p::str(v, "text").is_some() {
                wm.picture = None;
            }
            None
        }
    };
    if let Some((bytes, key)) = picked {
        let media = match key {
            Some(key) => key,
            None => {
                let ext = super::insert::image_ext(&bytes);
                s.doc.add_media(bytes, ext)
            }
        };
        wm.picture = Some(WatermarkPicture { media, ..wm.picture.take().unwrap_or_default() });
    }
    if let Some(t) = p::str(v, "text") {
        wm.text = t.chars().take(200).collect();
    }
    match wm.picture.as_mut() {
        Some(pic) => {
            if let Some(k) = ["diagonal", "color", "transparency", "semitransparent"].into_iter().find(|k| v.get(*k).is_some()) {
                return Err(CmdError::Params(format!("`{k}` is for a text watermark: give `text` or `\"picture\": false` too")));
            }
            if let Some(scale) = scale {
                pic.scale = scale;
            }
            if let Some(w) = p::bool(v, "washout") {
                pic.washout = w;
            }
        }
        None => {
            if let Some(k) = ["scale", "washout"].into_iter().find(|k| v.get(*k).is_some()) {
                return Err(CmdError::Params(format!("`{k}` is for a picture watermark: give `picture` too")));
            }
            if let Some(d) = p::bool(v, "diagonal") {
                wm.diagonal = d;
            }
            if let Some(c) = p::str(v, "color").and_then(Rgb::parse) {
                wm.color = c;
            }
            if let Some(on) = p::bool(v, "semitransparent") {
                wm.set_transparency(if on { 50 } else { 0 });
            }
            // Applied after `semitransparent`, so a percentage wins when both are given.
            if let Some(t) = p::f32(v, "transparency") {
                wm.set_transparency(if t.is_finite() { t.clamp(0.0, 100.0).round() as u8 } else { 0 });
            }
        }
    }
    s.doc.settings.watermark = Some(wm);
    sel_result(s)
}

/// Largest watermark picture taken, bytes. A watermark is a faded backdrop, not a photograph
/// library: far below Insert › Pictures' 200 MB, so refusing a bigger one never costs a large
/// allocation (the base64 text is measured before it is decoded).
const MAX_PICTURE_BYTES: usize = 64 << 20;

/// The picture of a watermark: a file (`path`), base64 `data`, or a picture already in the
/// document (`media`, returned as its key). Only pictures the renderers draw are taken.
fn watermark_picture(s: &Session, v: &Value) -> Result<(Vec<u8>, Option<String>), CmdError> {
    let unsupported = || CmdError::Failed("not a supported picture (PNG, JPEG, GIF, WebP, BMP, TIFF)".into());
    let readable = |b: &[u8]| wordcraft_doc::watermark::own_size(b).is_some();
    let too_large = || CmdError::Failed(format!("the picture is larger than {} MB", MAX_PICTURE_BYTES >> 20));
    if let Some(key) = p::str(v, "media") {
        let bytes = s.doc.media.get(key).ok_or_else(|| CmdError::Params(format!("no picture `{key}` in the document")))?;
        return if readable(bytes) { Ok((Vec::new(), Some(key.to_string()))) } else { Err(unsupported()) };
    }
    let bytes = if let Some(d) = p::str(v, "data") {
        // base64 is 4 characters for 3 bytes: measured before anything is decoded.
        if d.len() > MAX_PICTURE_BYTES / 3 * 4 + 4 {
            return Err(too_large());
        }
        super::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?
    } else if let Some(path) = p::str(v, "path") {
        crate::io::read_file_capped(path, MAX_PICTURE_BYTES as u64, "picture").map_err(CmdError::Failed)?
    } else {
        return Err(CmdError::Params("`picture` needs `path`, `data` or `media`".into()));
    };
    if bytes.len() > MAX_PICTURE_BYTES {
        return Err(too_large());
    }
    if !readable(&bytes) {
        return Err(unsupported());
    }
    Ok((bytes, None))
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::{Document, Watermark, WatermarkPicture};

    use super::MAX_PICTURE_BYTES;
    use crate::Session;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([30, 60, 200, 255]))
            .write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)
            .unwrap();
        b
    }

    fn wm(s: &Session) -> Watermark {
        s.doc.settings.watermark.clone().unwrap()
    }

    /// #2954: a picture watermark from a file's bytes, its scale and Washout, then back to text;
    /// each step undoes.
    #[test]
    fn picture_watermark_and_back_to_text() {
        let mut s = Session::new(Document::new());
        let data = super::super::insert::base64_encode(&png(40, 20));
        s.run("design.watermark", &json!({"picture": {"data": data}})).unwrap();
        let pic = wm(&s).picture.unwrap();
        assert_eq!((pic.scale, pic.washout), (None, true), "Word's defaults: Auto, washed out");
        assert!(s.doc.media.contains_key(&pic.media));
        s.run("design.watermark", &json!({"scale": 150, "washout": false})).unwrap();
        assert_eq!(wm(&s).picture, Some(WatermarkPicture { media: pic.media.clone(), scale: Some(150.0), washout: false }));
        s.run("design.watermark", &json!({"scale": "AUTO"})).unwrap();
        assert_eq!(wm(&s).picture.unwrap().scale, None);
        // Clamped to Word's range.
        s.run("design.watermark", &json!({"scale": 9000})).unwrap();
        assert_eq!(wm(&s).picture.unwrap().scale, Some(500.0));
        s.run("design.watermark", &json!({"scale": 0})).unwrap();
        assert_eq!(wm(&s).picture.unwrap().scale, Some(1.0));
        s.run("design.watermark", &json!({"scale": 33.333})).unwrap();
        assert_eq!(wm(&s).picture.unwrap().scale, Some(33.3));
        // A picture already in the document; text settings are refused while it is a picture.
        s.run("design.watermark", &json!({"picture": {"media": pic.media}, "scale": 50})).unwrap();
        assert!(s.run("design.watermark", &json!({"transparency": 30})).is_err());
        assert!(s.run("design.watermark", &json!({"diagonal": false})).is_err());
        // Text (or `picture: false`) makes it a text watermark again, with its text settings.
        s.run("design.watermark", &json!({"text": "SAMPLE", "transparency": 30})).unwrap();
        let text = wm(&s);
        assert_eq!((text.picture.is_none(), text.text.as_str(), text.transparency()), (true, "SAMPLE", 30));
        assert!(s.run("design.watermark", &json!({"washout": true})).is_err());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(wm(&s).picture.unwrap().scale, Some(50.0));
        s.run("design.watermark", &json!({"picture": false})).unwrap();
        assert_eq!(wm(&s).picture, None);
        s.run("design.watermark", &json!({"remove": true})).unwrap();
        assert_eq!(s.doc.settings.watermark, None);
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.settings.watermark.is_some());
    }

    /// #2954: the text's transparency in percent, kept in step with Semitransparent.
    #[test]
    fn text_watermark_transparency() {
        let mut s = Session::new(Document::new());
        s.run("design.watermark", &json!({"text": "DRAFT"})).unwrap();
        assert_eq!((wm(&s).transparency(), wm(&s).semitransparent), (50, true));
        s.run("design.watermark", &json!({"transparency": 80})).unwrap();
        assert_eq!((wm(&s).transparency(), wm(&s).semitransparent), (80, true));
        s.run("design.watermark", &json!({"semitransparent": false})).unwrap();
        assert_eq!((wm(&s).transparency(), wm(&s).semitransparent), (0, false));
        s.run("design.watermark", &json!({"transparency": 250.7})).unwrap();
        assert_eq!(wm(&s).transparency(), 100);
        s.run("design.watermark", &json!({"transparency": -3})).unwrap();
        assert_eq!(wm(&s).transparency(), 0);
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(wm(&s).transparency(), 100);
    }

    /// #2954: pictures that can't be drawn, and missing ones, are refused and change nothing.
    #[test]
    fn bad_watermark_pictures_are_refused() {
        let mut s = Session::new(Document::new());
        let text = s.doc.add_media(b"not a picture".to_vec(), "png");
        for bad in [
            json!({"picture": {}}),
            json!({"picture": {"data": "%%%"}}),
            json!({"picture": {"data": super::super::insert::base64_encode(b"plain text")}}),
            json!({"picture": {"media": "nope.png"}}),
            json!({"picture": {"media": text}}),
            json!({"picture": {"path": "/nonexistent/watermark.png"}}),
            json!({"picture": 7}),
            json!({"picture": {"data": super::super::insert::base64_encode(&png(4, 4))}, "scale": "big"}),
        ] {
            assert!(s.run("design.watermark", &bad).is_err(), "{bad}");
            assert_eq!(s.doc.settings.watermark, None, "{bad}");
            assert_eq!(s.doc.media.len(), 1, "{bad}");
        }
    }

    /// #2954 review: a picture too large is refused before anything is decoded or read: the
    /// base64 text is measured first, and a file by its size.
    #[test]
    fn oversized_watermark_pictures_are_refused_without_a_large_allocation() {
        let mut s = Session::new(Document::new());
        // Longer than the largest picture's base64 text: refused by length, never decoded (the
        // decoded bytes would be as large as the cap again).
        let big = "A".repeat(MAX_PICTURE_BYTES / 3 * 4 + 8);
        let e = s.run("design.watermark", &json!({"picture": {"data": big}})).unwrap_err();
        let e = format!("{e:?}");
        assert!(e.contains("larger than"), "{e}");
        assert_eq!((s.doc.settings.watermark.clone(), s.doc.media.len()), (None, 0));
        // A file past the cap is refused by its size (a sparse file, so nothing is written).
        let path = std::env::temp_dir().join(format!("wordcraft-wm-big-{}.png", std::process::id()));
        std::fs::File::create(&path).unwrap().set_len(MAX_PICTURE_BYTES as u64 + 1).unwrap();
        let e = s.run("design.watermark", &json!({"picture": {"path": path.to_string_lossy()}})).unwrap_err();
        let e = format!("{e:?}");
        let _ = std::fs::remove_file(&path);
        assert!(e.contains("larger than"), "{e}");
        assert_eq!((s.doc.settings.watermark.clone(), s.doc.media.len()), (None, 0));
    }
}
