use super::{Dialog, Value, WordApp, json};

/// The Font dialog preview's size: the Size box's points between 6 and 72, else 12. `"NaN"` and
/// `"inf"` parse as floats too, and a NaN font size trips epaint's glyph assertion (#3663).
pub(super) fn preview_size(size: &str) -> f32 {
    size.trim().parse::<f32>().ok().filter(|pt| pt.is_finite()).unwrap_or(12.0).clamp(6.0, 72.0)
}

/// The page colour the sample sits on.
pub(super) fn sample_paper(app: &WordApp) -> wordcraft_doc::Rgb {
    app.session.doc.settings.page_color.unwrap_or(wordcraft_doc::Rgb(255, 255, 255))
}

/// The sample's character properties: the dialog's fields as the document reads them.
#[allow(clippy::too_many_arguments)]
pub(super) fn font_sample_props(
    app: &mut WordApp,
    paper: wordcraft_doc::Rgb,
    font: &str,
    size: &str,
    color: &str,
    (bold, italic, underline, strike): (bool, bool, bool, bool),
    (small_caps, caps, hidden): (bool, bool, bool),
    spacing: f32,
) -> wordcraft_doc::props::CharProps {
    use wordcraft_doc::props::{CharProps, TextColor, Underline};
    // Automatic text is black, or white on a dark page.
    let dark = u32::from(paper.0) * 299 + u32::from(paper.1) * 587 + u32::from(paper.2) * 114 < 128_000;
    let auto = if dark { 255 } else { 0 };
    let ink = wordcraft_doc::Rgb::parse(color).unwrap_or(wordcraft_doc::Rgb(auto, auto, auto));
    // A hidden run is dotted, as Word draws hidden text when it is shown.
    let line = if hidden {
        Underline::Dotted
    } else if underline {
        Underline::Single
    } else {
        Underline::None
    };
    CharProps {
        font: Some(app.previews.font_stored(font)),
        size: Some(preview_size(size).min(40.0)),
        bold: Some(bold),
        italic: Some(italic),
        underline: Some(line),
        strike: Some(strike),
        color: Some(TextColor::Rgb(ink)),
        caps: Some(caps),
        small_caps: Some(small_caps),
        spacing: Some(spacing),
        ..Default::default()
    }
}

/// Draws the Font dialog's sample. A raised or lowered "2" follows the text
/// when Superscript or Subscript is on.
pub(super) fn font_sample_box(
    app: &mut WordApp,
    ui: &mut Ui,
    paper: wordcraft_doc::Rgb,
    props: &wordcraft_doc::props::CharProps,
    (sup, sub): (bool, bool),
    hidden: bool,
) {
    use wordcraft_doc::props::VertAlign;
    let dark = app.session.view.dark_mode;
    let mut render_options = crate::canvas::screen_render_options();
    render_options.dark = dark;
    render_options.dark_paper = wordcraft_render::DARK_PAPER;
    let display_paper = render_options.paper_of(&app.session.doc);
    let mark = if sup {
        Some(VertAlign::Superscript)
    } else if sub {
        Some(VertAlign::Subscript)
    } else {
        None
    };
    let (w, h) = (ui.available_width().clamp(120.0, 400.0), 56.0);
    let ppp = ui.ctx().pixels_per_point();
    let text = tl!("AaBbYyZz");
    let key = format!("fontsample:{text}:{props:?}:{mark:?}:{paper:?}:{display_paper:?}:{dark}:{w}:{ppp}");
    let tex = app.previews.get_or(ui.ctx(), &key, || crate::previews::font_sample(text, props, mark, w, h, ppp, paper, dark));
    let (r, _) = ui.allocate_exact_size(vec2(w + 4.0, h + 2.0), egui::Sense::hover());
    let t = crate::theme::Tokens::get(ui.ctx());
    let page = crate::theme::c32(display_paper);
    ui.painter().rect(r, 6.0, page, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    if let Some(tex) = tex {
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(tex.id(), egui::Rect::from_min_size(r.min, tex.size_vec2() / ppp), full, egui::Color32::WHITE);
    }
    if hidden {
        ui.label(egui::RichText::new(tl!("Hidden text is not shown unless you display it.")).small().weak());
    }
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
    let Dialog::Font { start, .. } = d else { return };
    let mut changed: serde_json::Map<String, Value> = font_props(d).into_iter().filter(|(k, v)| start.get(k) != Some(v)).collect();
    if let Some(f) = changed.get("font").and_then(Value::as_str).map(str::to_string) {
        let f = app.previews.font_stored(&f);
        changed.insert("font".into(), json!(f));
        changed.insert("fontCs".into(), json!(f));
    }
    if let Some(f) = changed.get("fontEa").and_then(Value::as_str).map(str::to_string) {
        changed.insert("fontEa".into(), json!(app.previews.font_stored(&f)));
    }
    if !changed.is_empty() {
        let _ = app.run("format.set", json!({"props": changed}));
    }
}
