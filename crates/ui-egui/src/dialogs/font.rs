use super::{Dialog, Value, WordApp, json};

/// The Font dialog preview's size: the Size box's points between 6 and 72, else 12. `"NaN"` and
/// `"inf"` parse as floats too, and a NaN font size trips epaint's glyph assertion (#3663).
pub(super) fn preview_size(size: &str) -> f32 {
    size.trim().parse::<f32>().ok().filter(|pt| pt.is_finite()).unwrap_or(12.0).clamp(6.0, 72.0)
}

/// The character properties the Font dialog's fields stand for.
pub(super) fn font_props(d: &Dialog) -> serde_json::Map<String, Value> {
    let Dialog::Font {
        font, font_ea, size, bold, italic, underline, strike, sup, sub, small_caps, caps, hidden, color, spacing, kern, kern_at, ..
    } = d
    else {
        return serde_json::Map::new();
    };
    let mut props = serde_json::Map::new();
    props.insert("bold".into(), json!(*bold));
    props.insert("italic".into(), json!(*italic));
    props.insert("strike".into(), json!(*strike));
    props.insert("underline".into(), json!(if *underline { "single" } else { "none" }));
    let vert = if *sup {
        "superscript"
    } else if *sub {
        "subscript"
    } else {
        "baseline"
    };
    props.insert("vertAlign".into(), json!(vert));
    props.insert("smallCaps".into(), json!(*small_caps));
    props.insert("caps".into(), json!(*caps));
    props.insert("hidden".into(), json!(*hidden));
    props.insert("spacing".into(), json!(*spacing));
    match kern {
        Some(true) => {
            props.insert("kern".into(), json!(kern_at.clamp(1.0, 1638.0)));
        }
        Some(false) => {
            props.insert("kern".into(), json!(0.0));
        }
        None => {}
    }
    if !font.trim().is_empty() {
        props.insert("font".into(), json!(font.trim()));
    }
    if !font_ea.trim().is_empty() {
        props.insert("fontEa".into(), json!(font_ea.trim()));
    }
    if let Ok(s) = size.trim().parse::<f64>() {
        props.insert("size".into(), json!(s));
    }
    if let Some(c) = wordcraft_doc::Rgb::parse(color) {
        props.insert("color".into(), json!({"Rgb": [c.0, c.1, c.2]}));
    } else if color.eq_ignore_ascii_case("auto") {
        // Automatic, from the colour field's drop-down (#606).
        props.insert("color".into(), json!("Auto"));
    }
    props
}

/// Is `c` Chinese, Japanese or Korean text (ideographs, kana, Hangul, CJK punctuation, fullwidth
/// forms)?
pub(super) fn east_asian_char(c: char) -> bool {
    matches!(u32::from(c), 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// The Font dialog's OK: one formatting step that sets only the properties the user changed, so
/// formatting the dialog can't show or left alone stays as it is. Fonts are set by their stored
/// (English) names, the Latin font for Persian/Arabic text too (as `format.font`).
pub(super) fn apply_font(app: &mut WordApp, d: &Dialog) {
    let changed = changed_font_props(app, d);
    if !changed.is_empty() {
        let _ = app.run("format.set", json!({"props": changed}));
    }
}

/// The properties the user changed in the Font dialog, fonts by their stored names.
fn changed_font_props(app: &mut WordApp, d: &Dialog) -> serde_json::Map<String, Value> {
    let Dialog::Font { start, .. } = d else { return serde_json::Map::new() };
    let mut changed: serde_json::Map<String, Value> = font_props(d).into_iter().filter(|(k, v)| start.get(k) != Some(v)).collect();
    if let Some(f) = changed.get("font").and_then(Value::as_str).map(str::to_string) {
        let f = app.previews.font_stored(&f);
        changed.insert("font".into(), json!(f));
        changed.insert("fontCs".into(), json!(f));
    }
    if let Some(f) = changed.get("fontEa").and_then(Value::as_str).map(str::to_string) {
        changed.insert("fontEa".into(), json!(app.previews.font_stored(&f)));
    }
    changed
}

/// Define New Number Format › Font… (#2787): the Font dialog for a list's number, its fields
/// filled from `props` (the number's properties so far, in `format.set`'s shape). Its OK formats
/// no text; [`number_font_result`] gives what changed.
pub(crate) fn number_font_dialog(app: &mut WordApp, props: &serde_json::Map<String, Value>) -> Option<Dialog> {
    let mut d = Dialog::open("font", app)?;
    let b = |k: &str| props.get(k).and_then(Value::as_bool).unwrap_or(false);
    let shown = app.previews.font_shown(props.get("font").and_then(Value::as_str).unwrap_or(""));
    if let Dialog::Font {
        font,
        font_was,
        font_ea,
        font_ea_was,
        asian,
        size,
        bold,
        italic,
        underline,
        strike,
        sup,
        sub,
        small_caps,
        caps,
        hidden,
        color,
        spacing,
        for_number,
        ..
    } = &mut d
    {
        (*font, *font_was) = (shown.clone(), shown);
        (*font_ea, *font_ea_was, *asian) = (String::new(), String::new(), false);
        *size = props.get("size").and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_default();
        (*bold, *italic, *strike, *small_caps, *caps, *hidden) = (b("bold"), b("italic"), b("strike"), b("smallCaps"), b("caps"), b("hidden"));
        *underline = props.get("underline").and_then(Value::as_str).is_some_and(|u| u != "none");
        let vert = props.get("vertAlign").and_then(Value::as_str);
        (*sup, *sub) = (vert == Some("superscript"), vert == Some("subscript"));
        *spacing = props.get("spacing").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        *color = match props.get("color").and_then(|c| c.get("Rgb")).and_then(Value::as_array) {
            Some(c) => c.iter().filter_map(Value::as_u64).map(|x| format!("{:02X}", x.min(255))).collect(),
            None => "auto".into(),
        };
        *for_number = Some(false);
    }
    let start = font_props(&d);
    if let Dialog::Font { start: s, .. } = &mut d {
        *s = start;
    }
    Some(d)
}

/// Draws a number's Font dialog (from [`number_font_dialog`]) in the same window as every dialog,
/// over the one that opened it: `Some(true)` on OK, `Some(false)` on Cancel, Escape or its close
/// button, `None` while it stays open.
pub(crate) fn show_number_font(app: &mut WordApp, ctx: &egui::Context, d: &mut Dialog) -> Option<bool> {
    let (mut open, mut close) = (true, false);
    egui::Window::new(tl!("Font"))
        .id(egui::Id::unique(("dialog", "Font", "number")))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_min_width(340.0);
            close = super::dispatch::body(app, ui, d);
        });
    if open && !close {
        return None;
    }
    Some(matches!(d, Dialog::Font { for_number: Some(true), .. }))
}

/// After a number's Font dialog closed: the properties changed on OK, `None` on Cancel.
pub(crate) fn number_font_result(app: &mut WordApp, d: &Dialog) -> Option<serde_json::Map<String, Value>> {
    matches!(d, Dialog::Font { for_number: Some(true), .. }).then(|| changed_font_props(app, d))
}
