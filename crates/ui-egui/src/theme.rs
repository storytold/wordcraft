//! Design tokens (light and dark), UI fonts and the egui style.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Rect, Stroke, Visuals};

/// WordCraft's app colour (and its darker ink for text on light backgrounds).
pub const APP_COLOR: Color32 = Color32::from_rgb(0x3B, 0x5B, 0xDB);
pub const APP_INK: Color32 = Color32::from_rgb(0x2B, 0x47, 0xB5);

/// The interface theme setting (File › Options › General, View › Dark Mode): a fixed light or
/// dark palette, or `System`, which follows the OS light/dark appearance live (#115).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Appearance {
    #[default]
    Light,
    Dark,
    System,
}

impl Appearance {
    /// In menu order.
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    /// The value used in `ui.json` and by the `ui.theme` command.
    pub fn code(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::System => "system",
        }
    }

    pub fn from_code(code: &str) -> Option<Appearance> {
        Appearance::ALL.into_iter().find(|a| a.code().eq_ignore_ascii_case(code.trim()))
    }

    /// The English label (translated with `tl!` where shown).
    pub fn label(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
            Appearance::System => "Use system setting",
        }
    }

    /// Whether the interface is dark, given the OS appearance egui reports (`Context::system_theme`).
    /// `System` falls back to light when the OS reports none.
    pub fn is_dark(self, system: Option<egui::Theme>) -> bool {
        match self {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::System => system == Some(egui::Theme::Dark),
        }
    }
}

/// An unknown saved value reads as the default rather than discarding the whole `ui.json`.
impl From<String> for Appearance {
    fn from(s: String) -> Self {
        Appearance::from_code(&s).unwrap_or_default()
    }
}

impl From<Appearance> for String {
    fn from(a: Appearance) -> Self {
        a.code().to_string()
    }
}

/// Every colour the UI uses; widgets never hard-code colours.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub title_bar: Color32,
    pub ribbon: Color32,
    pub tab_strip: Color32,
    pub canvas: Color32,
    pub panel: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub icon: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub checked: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    /// Text/icon colour drawn on top of `accent` (e.g. keytip badge letters).
    pub on_accent: Color32,
    pub input: Color32,
    pub input_border: Color32,
    pub group_label: Color32,
    pub ruler: Color32,
    pub ruler_margin: Color32,
    pub ruler_tick: Color32,
    pub status_bar: Color32,
    pub selection: Color32,
    pub caret: Color32,
    pub page_shadow: Color32,
    /// Selection-frame handles (pictures, shapes, text boxes) and their shadow.
    pub handle: Color32,
    pub handle_shadow: Color32,
    pub menu: Color32,
    pub blue: Color32,
    pub red: Color32,
    pub green: Color32,
    pub orange: Color32,
}

impl Tokens {
    pub fn light() -> Self {
        Tokens {
            dark: false,
            title_bar: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            ribbon: Color32::from_rgb(0xF9, 0xF9, 0xF9),
            tab_strip: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            canvas: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            panel: Color32::from_rgb(0xFA, 0xFA, 0xFA),
            border: Color32::from_rgb(0xE1, 0xE1, 0xE1),
            border_strong: Color32::from_rgb(0xC8, 0xC8, 0xC8),
            text: Color32::from_rgb(0x24, 0x24, 0x24),
            text_dim: Color32::from_rgb(0x61, 0x61, 0x61),
            text_disabled: Color32::from_rgb(0xA8, 0xA8, 0xA8),
            icon: Color32::from_rgb(0x42, 0x42, 0x42),
            hover: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            pressed: Color32::from_rgb(0xD6, 0xD6, 0xD6),
            checked: Color32::from_rgb(0xDD, 0xE3, 0xF8),
            accent: APP_COLOR,
            accent_text: APP_INK,
            on_accent: Color32::WHITE,
            input: Color32::WHITE,
            input_border: Color32::from_rgb(0xC7, 0xC7, 0xC7),
            group_label: Color32::from_rgb(0x70, 0x70, 0x70),
            ruler: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            ruler_margin: Color32::from_rgb(0xD4, 0xD4, 0xD4),
            ruler_tick: Color32::from_rgb(0x70, 0x70, 0x70),
            status_bar: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            selection: Color32::from_rgba_unmultiplied(0x3B, 0x5B, 0xDB, 0x48),
            caret: Color32::BLACK,
            page_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x22),
            handle: Color32::WHITE,
            handle_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x28),
            menu: Color32::WHITE,
            blue: Color32::from_rgb(0x2F, 0x6F, 0xD0),
            red: Color32::from_rgb(0xD1, 0x3B, 0x3B),
            green: Color32::from_rgb(0x2E, 0x8B, 0x57),
            orange: Color32::from_rgb(0xE0, 0x7B, 0x1F),
        }
    }
    pub fn dark() -> Self {
        Tokens {
            dark: true,
            title_bar: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            ribbon: Color32::from_rgb(0x29, 0x29, 0x29),
            tab_strip: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            canvas: Color32::from_rgb(0x14, 0x14, 0x14),
            panel: Color32::from_rgb(0x26, 0x26, 0x26),
            border: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            border_strong: Color32::from_rgb(0x55, 0x55, 0x55),
            text: Color32::from_rgb(0xEE, 0xEE, 0xEE),
            text_dim: Color32::from_rgb(0xC8, 0xC8, 0xC8),
            text_disabled: Color32::from_rgb(0x6A, 0x6A, 0x6A),
            icon: Color32::from_rgb(0xDD, 0xDD, 0xDD),
            hover: Color32::from_rgb(0x3D, 0x3D, 0x3D),
            pressed: Color32::from_rgb(0x4A, 0x4A, 0x4A),
            checked: Color32::from_rgb(0x2E, 0x3A, 0x66),
            accent: Color32::from_rgb(0x7A, 0x93, 0xF0),
            accent_text: Color32::from_rgb(0x9C, 0xB0, 0xF5),
            on_accent: Color32::WHITE,
            input: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            input_border: Color32::from_rgb(0x58, 0x58, 0x58),
            group_label: Color32::from_rgb(0xA0, 0xA0, 0xA0),
            ruler: Color32::from_rgb(0x2E, 0x2E, 0x2E),
            ruler_margin: Color32::from_rgb(0x45, 0x45, 0x45),
            ruler_tick: Color32::from_rgb(0xA0, 0xA0, 0xA0),
            status_bar: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            selection: Color32::from_rgba_unmultiplied(0x7A, 0x93, 0xF0, 0x55),
            caret: Color32::BLACK,
            page_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x60),
            handle: Color32::from_rgb(0xF2, 0xF2, 0xF2),
            handle_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x5A),
            menu: Color32::from_rgb(0x2B, 0x2B, 0x2B),
            blue: Color32::from_rgb(0x6C, 0x9B, 0xEA),
            red: Color32::from_rgb(0xEE, 0x6A, 0x6A),
            green: Color32::from_rgb(0x5B, 0xC0, 0x8A),
            orange: Color32::from_rgb(0xF0, 0xA0, 0x50),
        }
    }
    pub fn get(ctx: &egui::Context) -> Tokens {
        if ctx.global_style().visuals.dark_mode { Tokens::dark() } else { Tokens::light() }
    }
}

/// Install the UI fonts (Inter, JetBrains Mono) and the craft-fonts CJK interface faces if built
/// in, Japanese first.
pub fn install_fonts(ctx: &egui::Context) {
    install_fonts_for(ctx, false);
}

/// [`install_fonts`] with the CJK fallback order for the interface language: the Chinese face
/// first for Chinese (#8), the Japanese one otherwise. The new fonts apply from the next frame.
pub fn install_fonts_for(ctx: &egui::Context, prefer_hans: bool) {
    ctx.set_fonts(font_definitions(prefer_hans));
}

/// The interface fonts; see [`install_fonts_for`].
pub fn font_definitions(prefer_hans: bool) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "Inter", include_bytes!("../../../assets/fonts/Inter-Regular.ttf"));
    add(&mut fonts, "InterMedium", include_bytes!("../../../assets/fonts/Inter-Medium.ttf"));
    add(&mut fonts, "InterSemiBold", include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"));
    add(&mut fonts, "SourceSans", include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf"));
    add(&mut fonts, "Mono", include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"));
    if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
        prop.insert(0, "Inter".into());
        prop.push("SourceSans".into());
    }
    if let Some(m) = fonts.families.get_mut(&FontFamily::Monospace) {
        m.insert(0, "Mono".into());
    }
    fonts.families.insert(FontFamily::Name("medium".into()), vec!["InterMedium".into(), "Inter".into(), "SourceSans".into()]);
    fonts.families.insert(FontFamily::Name("semibold".into()), vec!["InterSemiBold".into(), "Inter".into(), "SourceSans".into()]);
    let add_cjk = |fonts: &mut FontDefinitions, name: String, data: FontData| {
        fonts.font_data.insert(name.clone(), Arc::new(data));
        for fam in [FontFamily::Proportional, FontFamily::Name("medium".into()), FontFamily::Name("semibold".into())] {
            if let Some(v) = fonts.families.get_mut(&fam) {
                v.push(name.clone());
            }
        }
    };
    let cjk = wordcraft_fonts::ui_cjk_fonts(prefer_hans);
    for f in &cjk {
        // The same static bytes the document fonts use: one copy in the binary.
        add_cjk(&mut fonts, format!("{} {}", f.family, f.style), FontData::from_static(f.bytes));
    }
    // No embedded face covers the interface language (a build without craft-fonts, or without
    // its Chinese face, #241): an installed CJK font, so the menus don't show boxes.
    #[cfg(not(target_arch = "wasm32"))]
    if wordcraft_fonts::ui_needs_system_cjk(prefer_hans, &cjk)
        && let Some(f) = system_cjk_font(prefer_hans)
    {
        let mut data = FontData::from_static(&f.bytes);
        data.index = f.index;
        add_cjk(&mut fonts, format!("system {}", f.family), data);
    }
    // Symbols and emoji fall back to egui's defaults (kept in the families).
    fonts
}

/// The installed CJK interface font for a Chinese (`hans`) or other interface, read once.
#[cfg(not(target_arch = "wasm32"))]
fn system_cjk_font(hans: bool) -> Option<&'static wordcraft_fonts::SystemUiFont> {
    static ZH: std::sync::OnceLock<Option<wordcraft_fonts::SystemUiFont>> = std::sync::OnceLock::new();
    static OTHER: std::sync::OnceLock<Option<wordcraft_fonts::SystemUiFont>> = std::sync::OnceLock::new();
    (if hans { &ZH } else { &OTHER }).get_or_init(|| wordcraft_fonts::system_cjk_ui_font(hans)).as_ref()
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// A UI type rung: size, weight pairing, and size-specific letter-spacing (large text is
/// tightened, small text opened). Use these instead of raw `regular(size)`/`semibold(size)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TypeRung {
    /// Tab strip, title bar, centred window title.
    Chrome,
    /// Ribbon buttons, menus, dialogs.
    Control,
    /// Group labels, status bar, tooltips.
    Caption,
}

impl TypeRung {
    /// Font size in points.
    pub fn size(self) -> f32 {
        match self {
            TypeRung::Chrome => 12.5,
            TypeRung::Control => 11.5,
            TypeRung::Caption => 11.0,
        }
    }
    /// Extra letter-spacing in points (em-relative, so it scales with the size).
    pub fn tracking(self) -> f32 {
        match self {
            TypeRung::Chrome => -0.012 * self.size(),
            TypeRung::Control => 0.0,
            TypeRung::Caption => 0.018 * self.size(),
        }
    }
    /// Line height in points; tighter leading as the text grows.
    pub fn line_height(self) -> f32 {
        match self {
            TypeRung::Chrome => 1.25 * self.size(),
            TypeRung::Control => 1.42 * self.size(),
            TypeRung::Caption => 1.5 * self.size(),
        }
    }

    pub fn regular(self) -> FontId {
        regular(self.size())
    }
    pub fn medium(self) -> FontId {
        medium(self.size())
    }
    pub fn semibold(self) -> FontId {
        semibold(self.size())
    }
}

/// Width in points of one line of `tracking`-spaced text. `layout_no_wrap` ignores
/// letter-spacing, so non-zero tracking goes through an explicit `LayoutJob`.
pub fn text_width(painter: &egui::Painter, text: &str, font: &FontId, tracking: f32, color: Color32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    if tracking.abs() <= 1e-6 {
        return painter.layout_no_wrap(text.to_string(), font.clone(), color).size().x;
    }
    painter.layout_job(tracked_job(text, font, tracking, color)).size().x
}

/// Paint one line of tracked text with the given anchor and return its rect.
pub fn paint_text(painter: &egui::Painter, pos: egui::Pos2, anchor: Align2, text: &str, font: &FontId, tracking: f32, color: Color32) -> Rect {
    if tracking.abs() <= 1e-6 {
        return painter.text(pos, anchor, text.to_string(), font.clone(), color);
    }
    let galley = painter.layout_job(tracked_job(text, font, tracking, color));
    let rect = anchor.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// A single-section, no-wrap `LayoutJob` carrying `tracking` (egui 0.36 has no `default_format`).
fn tracked_job(text: &str, font: &FontId, tracking: f32, color: Color32) -> egui::text::LayoutJob {
    let format = egui::text::TextFormat { font_id: font.clone(), extra_letter_spacing: tracking, line_height: None, color, ..Default::default() };
    egui::text::LayoutJob::single_section(text.to_string(), format)
}

/// Apply tokens to egui's style.
pub fn apply(ctx: &egui::Context, t: &Tokens) {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.ribbon;
    v.window_fill = t.menu;
    v.extreme_bg_color = t.input;
    v.faint_bg_color = t.panel;
    v.window_stroke = Stroke::new(1.0, t.border_strong);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = t.selection;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent_text;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.bg_fill = t.input;
    v.widgets.inactive.weak_bg_fill = t.input;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.corner_radius = CornerRadius::same(4);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.corner_radius = CornerRadius::same(4);
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.weak_bg_fill = t.pressed;
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.active.corner_radius = CornerRadius::same(4);
    v.widgets.open.bg_fill = t.pressed;
    v.widgets.open.weak_bg_fill = t.pressed;
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if t.dark { 120 } else { 40 }) };
    v.window_shadow = egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 50 }) };
    // Pin egui's own theme to ours: left on its default (follow the OS), egui would swap to its
    // other, unstyled light/dark style the moment the OS appearance changed.
    ctx.set_theme(if t.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size = egui::vec2(24.0, 22.0);
        s.spacing.menu_margin = egui::Margin::same(6);
        s.text_styles.insert(egui::TextStyle::Body, regular(12.5));
        s.text_styles.insert(egui::TextStyle::Button, regular(12.5));
        s.text_styles.insert(egui::TextStyle::Small, regular(10.5));
        s.text_styles.insert(egui::TextStyle::Heading, semibold(17.0));
        s.interaction.tooltip_delay = 0.45;
    });
}

pub fn c32(c: wordcraft_doc::Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}
