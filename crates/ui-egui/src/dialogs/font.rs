use super::{Dialog, Ui, Value, WordApp, json, vec2};

/// The most a character may be expanded or condensed, in points (Word's limit, and `w:spacing`'s
/// on reading).
pub(super) const MAX_CHAR_SPACING: f32 = 1584.0;

/// One click of the Spacing box's arrows, in points.
pub(super) const CHAR_SPACING_STEP: f32 = 0.5;

/// The Spacing box's choices: Normal, Expanded (positive spacing) and Condensed (negative).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpacingKind {
    Normal,
    Expanded,
    Condensed,
}

impl SpacingKind {
    pub(super) const ALL: [(SpacingKind, &'static str); 3] =
        [(SpacingKind::Normal, "Normal"), (SpacingKind::Expanded, "Expanded"), (SpacingKind::Condensed, "Condensed")];

    /// The choice a character spacing in points shows.
    pub(super) fn of(points: f32) -> SpacingKind {
        if points > 0.0 {
            SpacingKind::Expanded
        } else if points < 0.0 {
            SpacingKind::Condensed
        } else {
            SpacingKind::Normal
        }
    }

    /// The character spacing for this choice and a By: amount, 0 to [`MAX_CHAR_SPACING`].
    pub(super) fn points(self, by: f32) -> f32 {
        let by = if by.is_finite() { by.clamp(0.0, MAX_CHAR_SPACING) } else { 0.0 };
        match self {
            SpacingKind::Normal => 0.0,
            SpacingKind::Expanded => by,
            SpacingKind::Condensed => -by,
        }
    }
}

/// The By: amount after an arrow click (`up` or down): half a point, kept within 0 to 1584 pt
/// and on the half-point grid, so 1.3 steps to 1.5 or 1.0.
pub(super) fn step_char_spacing(by: f32, up: bool) -> f32 {
    let by = if by.is_finite() { by.clamp(0.0, MAX_CHAR_SPACING) } else { 0.0 };
    let units = by / CHAR_SPACING_STEP;
    let next = if up { (units + 1e-4).floor() + 1.0 } else { (units - 1e-4).ceil() - 1.0 };
    (next * CHAR_SPACING_STEP).clamp(0.0, MAX_CHAR_SPACING)
}

/// The Font dialog's Spacing box (Advanced): Normal, Expanded or Condensed, then By: the amount
/// in points (0 to 1584) with ▲ and ▼ arrows that step it by half a point. `spacing` is the
/// signed character spacing the dialog applies.
pub(super) fn char_spacing_field(ui: &mut Ui, spacing: &mut f32, label: &egui::Response) {
    let mut kind = SpacingKind::of(*spacing);
    let mut by = spacing.abs().min(MAX_CHAR_SPACING);
    ui.horizontal(|ui| {
        let shown = SpacingKind::ALL.iter().find(|(k, _)| *k == kind).map_or("Normal", |(_, l)| l);
        let combo = craft_ui::ComboBox::from_id_salt("fontdlg_spacing").selected_text(tl!(shown)).show_ui(ui, |ui| {
            for (k, l) in SpacingKind::ALL {
                if ui.selectable_label(kind == k, tl!(l)).clicked() && kind != k {
                    // Word starts Expanded and Condensed at 1 pt when the text had none.
                    if kind == SpacingKind::Normal && by == 0.0 {
                        by = 1.0;
                    }
                    kind = k;
                }
            }
        });
        combo.response.labelled_by(label.id);
        let enabled = kind != SpacingKind::Normal;
        let by_label = ui.label(tl!("By:"));
        let field = egui::DragValue::new(&mut by).speed(CHAR_SPACING_STEP).range(0.0..=MAX_CHAR_SPACING).max_decimals(2).suffix(" pt");
        ui.add_enabled(enabled, field).labelled_by(by_label.id);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let arrow = |ui: &mut Ui, text: &str, hint: &str| {
                ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(7.0)).min_size(vec2(16.0, 10.0)))
                    .on_hover_text(tl!(hint))
                    .clicked()
            };
            if arrow(ui, "▲", "Increase by 0.5 pt") {
                by = step_char_spacing(by, true);
            }
            if arrow(ui, "▼", "Decrease by 0.5 pt") {
                by = step_char_spacing(by, false);
            }
        });
    });
    *spacing = kind.points(by);
}

/// The Font dialog preview's size: the Size box's points between 6 and 72, else 12. `"NaN"` and
/// `"inf"` parse as floats too, and a NaN font size trips epaint's glyph assertion (#3663).
pub(super) fn preview_size(size: &str) -> f32 {
    size.trim().parse::<f32>().ok().filter(|pt| pt.is_finite()).unwrap_or(12.0).clamp(6.0, 72.0)
}

/// The page colour the sample sits on.
pub(super) fn sample_paper(app: &WordApp) -> wordcraft_doc::Rgb {
    app.session.doc.settings.page_color.unwrap_or(wordcraft_doc::Rgb(255, 255, 255))
}

const SAMPLE_WIDTH: f32 = 300.0;

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
    let line = if underline {
        Underline::Single
    } else if hidden {
        Underline::Dotted
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
    // A fixed width: sizing it from the dialog's width would grow the dialog frame by frame.
    let (w, h) = (SAMPLE_WIDTH, 56.0);
    let ppp = ui.ctx().pixels_per_point();
    let text = tl!("AaBbYyZz");
    let key = format!("fontsample:{text}:{props:?}:{mark:?}:{paper:?}:{display_paper:?}:{dark}:{w}:{ppp}");
    let tex = app.previews.get_or(ui.ctx(), &key, || crate::previews::font_sample(text, props, mark, (w, h, ppp), paper, dark));
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

/// One of the Font dialog's font boxes: type to find a font, with the missing-font warning
/// `shown_as` (the families drawn instead of `current`, see [`WordApp::font_dialog_warning`]).
pub(super) fn font_box(
    ui: &mut egui::Ui,
    (id, current): (&str, &str),
    items: &[String],
    preview: &dyn Fn(&mut egui::Ui, &str) -> egui::Response,
    shown_as: Option<&str>,
) -> Option<String> {
    let options =
        crate::widgets::FontComboOptions { id, width: 220.0, current, items, recent: &[], preview: Some(preview), tip: tl!("Font"), shown_as };
    crate::widgets::font_combo_with_recent_hint(ui, options)
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

#[cfg(test)]
mod char_spacing_tests {
    use super::{SpacingKind, step_char_spacing};

    #[test]
    fn the_spacing_arrows_step_half_a_point_within_0_to_1584() {
        assert_eq!(step_char_spacing(0.0, true), 0.5);
        assert_eq!(step_char_spacing(0.5, true), 1.0);
        assert_eq!(step_char_spacing(1.0, false), 0.5);
        assert_eq!(step_char_spacing(0.0, false), 0.0);
        assert_eq!(step_char_spacing(1.3, true), 1.5);
        assert_eq!(step_char_spacing(1.3, false), 1.0);
        assert_eq!(step_char_spacing(1584.0, true), 1584.0);
        assert_eq!(step_char_spacing(1583.5, true), 1584.0);
        assert_eq!(step_char_spacing(f32::NAN, true), 0.5);
        assert_eq!(step_char_spacing(5000.0, false), 1583.5);
    }

    #[test]
    fn expanded_and_condensed_set_the_sign_of_the_spacing() {
        assert_eq!(SpacingKind::of(2.0), SpacingKind::Expanded);
        assert_eq!(SpacingKind::of(-2.0), SpacingKind::Condensed);
        assert_eq!(SpacingKind::of(0.0), SpacingKind::Normal);
        assert_eq!(SpacingKind::Expanded.points(1584.0), 1584.0);
        assert_eq!(SpacingKind::Condensed.points(1584.0), -1584.0);
        assert_eq!(SpacingKind::Condensed.points(9999.0), -1584.0);
        assert_eq!(SpacingKind::Normal.points(3.0), 0.0);
        assert_eq!(SpacingKind::Expanded.points(f32::INFINITY), 0.0);
    }
}
