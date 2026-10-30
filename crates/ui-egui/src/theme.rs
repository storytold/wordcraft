//! Design tokens (light and dark), UI fonts and the egui style.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Rect, Stroke, Visuals};

/// WordCraft's app colour (and its darker ink for text on light backgrounds).
pub const APP_COLOR: Color32 = Color32::from_rgb(0x3B, 0x5B, 0xDB);
pub const APP_INK: Color32 = Color32::from_rgb(0x2B, 0x47, 0xB5);

/// The interface theme setting (File › Options › General, View › Interface Theme): a fixed light or
/// dark palette, or `System`, which follows the OS light/dark appearance live (#115). Dark page
/// (View › Switch Modes) is separate: it changes how pages are drawn, never the interface (#312).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Appearance {
    Light,
    Dark,
    #[default]
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

    /// The egui theme preference: `System` stays automatic rather than resolving to a fixed theme.
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Appearance::Light => egui::ThemePreference::Light,
            Appearance::Dark => egui::ThemePreference::Dark,
            Appearance::System => egui::ThemePreference::System,
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

/// Interface size (File › Options › General, `ui.interfaceSize`): how large the ribbon, menus, panes
/// and dialogs are drawn, through egui's zoom factor. Document zoom (View › Zoom) is a separate
/// setting and never changes with it.
///
/// `Auto` (the default) keeps the display scale the system reports. A chosen size is absolute, as in
/// PhotoCraft and PdfCraft (#822): "100%" is one screen pixel per point whatever the display reports,
/// so a system that guesses 200 % (a bare window manager on X11, where winit falls back to the
/// monitor's DPI) doesn't make "100%" draw at 200 %. The display's scale is divided back out in
/// [`InterfaceSize::zoom_factor`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "serde_json::Value", into = "serde_json::Value")]
pub struct InterfaceSize(Option<u16>);

impl InterfaceSize {
    pub const MIN: u16 = 50;
    pub const MAX: u16 = 300;
    /// Follow the display scale the system reports.
    pub const AUTO: InterfaceSize = InterfaceSize(None);
    /// The sizes File › Options offers after Auto, in percent (PhotoCraft's and PdfCraft's).
    pub const PRESETS: [u16; 12] = [75, 80, 85, 90, 95, 100, 125, 150, 175, 200, 250, 300];

    /// A chosen size from any number: rounded and clamped to 50–300 %; `None` for NaN or infinity.
    pub fn from_percent(percent: f64) -> Option<InterfaceSize> {
        percent.is_finite().then(|| InterfaceSize(Some(percent.round().clamp(f64::from(Self::MIN), f64::from(Self::MAX)) as u16)))
    }

    /// The chosen size in percent, or `None` for Auto.
    pub fn percent(self) -> Option<u16> {
        self.0
    }

    pub fn is_auto(self) -> bool {
        self.0.is_none()
    }

    /// The egui zoom factor on a display whose own scale (native pixels per point) is `native`:
    /// 1.0 for Auto, or the chosen size with the display's scale divided back out.
    pub fn zoom_factor(self, native: f32) -> f32 {
        match self.0 {
            None => 1.0,
            Some(pct) => f32::from(pct) / 100.0 / usable_scale(native),
        }
    }

    /// The size the interface is drawn at on that display, in percent (screen pixels per point × 100).
    pub fn drawn_percent(self, native: f32) -> u16 {
        let drawn = usable_scale(native) * self.zoom_factor(native) * 100.0;
        drawn.round().clamp(0.0, f32::from(u16::MAX)) as u16
    }
}

/// The display's scale, or 1.0 when egui doesn't know it (or reports nonsense).
pub fn usable_scale(native: f32) -> f32 {
    if native.is_finite() && native > 0.0 { native } else { 1.0 }
}

/// `"auto"`, or a percentage; anything else reads as Auto rather than discarding the whole `ui.json`.
impl From<serde_json::Value> for InterfaceSize {
    fn from(v: serde_json::Value) -> Self {
        v.as_f64().and_then(InterfaceSize::from_percent).unwrap_or_default()
    }
}

impl From<InterfaceSize> for serde_json::Value {
    fn from(s: InterfaceSize) -> Self {
        s.0.map_or_else(|| serde_json::Value::from("auto"), serde_json::Value::from)
    }
}

/// A `ui.json` from before #822 saved `interfaceSize` as a percentage on top of the display scale.
/// Its default, 100, meant "the display's scale", which is Auto now (`None`); another value was
/// picked on purpose and is returned, to become an absolute size once the display's scale is
/// known ([`migrate_legacy_interface_size`]). Junk reads as Auto.
pub fn legacy_interface_percent(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().filter(|p| p.is_finite() && (p - 100.0).abs() >= 0.5)
}

/// The absolute size a pre-#822 `interfaceSize` of `old` percent drew at on a display whose own
/// scale is `native`: `old × native`, clamped to 50–300 %. A 2× display with 125 % drew at 250 %,
/// and keeps drawing there.
pub fn migrate_legacy_interface_size(old: f64, native: f32) -> InterfaceSize {
    InterfaceSize::from_percent(old * f64::from(usable_scale(native))).unwrap_or_default()
}

/// The Animation effects setting (File › Options › Accessibility, `ui.animations`): on, off, or
/// `System`, which follows the system's own setting as the host reports it
/// ([`Services::animations_enabled`](crate::Services::animations_enabled): Windows' Animation
/// effects, macOS' Reduce motion, the Linux/BSD desktop's setting, a browser's
/// `prefers-reduced-motion`).
/// Off: wheel notches scroll and zoom at once ([`crate::scroll`]) and nothing fades or glides
/// ([`apply_animations`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Animations {
    #[default]
    System,
    On,
    Off,
}

impl Animations {
    /// In menu order.
    pub const ALL: [Animations; 3] = [Animations::System, Animations::On, Animations::Off];

    /// The value used in `ui.json` and by the `ui.animations` command.
    pub fn code(self) -> &'static str {
        match self {
            Animations::System => "system",
            Animations::On => "on",
            Animations::Off => "off",
        }
    }

    pub fn from_code(code: &str) -> Option<Animations> {
        Animations::ALL.into_iter().find(|a| a.code().eq_ignore_ascii_case(code.trim()))
    }

    /// The English label (translated with `tl!` where shown).
    pub fn label(self) -> &'static str {
        match self {
            Animations::System => "Use system setting",
            Animations::On => "On",
            Animations::Off => "Off",
        }
    }

    /// Whether animation effects are on, given what the system reports (`None`: it doesn't say).
    /// `System` falls back to on when the system reports nothing.
    pub fn enabled(self, system: Option<bool>) -> bool {
        match self {
            Animations::System => system.unwrap_or(true),
            Animations::On => true,
            Animations::Off => false,
        }
    }
}

/// An unknown saved value reads as the default rather than discarding the whole `ui.json`.
impl From<String> for Animations {
    fn from(s: String) -> Self {
        Animations::from_code(&s).unwrap_or_default()
    }
}

impl From<Animations> for String {
    fn from(a: Animations) -> Self {
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
    /// The File page's side bar; white text and icons sit on it in both themes.
    pub nav: Color32,
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
    /// Find › Reading Highlight.
    pub find_highlight: Color32,
    pub page_shadow: Color32,
    /// Selection-frame handles (pictures, shapes, text boxes) and their shadow.
    pub handle: Color32,
    pub handle_shadow: Color32,
    /// Alignment guides while moving a picture or shape: drawn on the page, so legible on white
    /// paper in both themes.
    pub guide: Color32,
    /// The frame around the equation the caret is in, its grip and its options strip: drawn on
    /// the page, so the same light blue in both themes.
    pub equation_box: Color32,
    /// Crop mode: the crop handles, and the dim over the cropped-away part of a picture.
    pub crop_handle: Color32,
    pub crop_dim: Color32,
    /// Comment and revision balloons in the markup area beside the page (#2045): the card, the
    /// area behind it and its edge. The card holds `text`, so it is dark in the dark theme.
    pub balloon: Color32,
    pub markup_area: Color32,
    pub markup_edge: Color32,
    /// "Resolved" on a balloon: `green` is only 4.2:1 on the light card.
    pub resolved_text: Color32,
    /// One stripe and leader colour per comment author, in turn.
    pub author_colors: [Color32; 5],
    /// The same colours for the leader line across the page: the paper stays white in the dark
    /// theme, so these are the light-theme colours in both.
    pub leader_colors: [Color32; 5],
    pub menu: Color32,
    pub blue: Color32,
    pub red: Color32,
    pub green: Color32,
    pub orange: Color32,
    /// A window's close box while the pointer is over it (the red of a Windows caption button),
    /// while pressed, and the ✕ drawn on it then (#1885).
    pub close_hover: Color32,
    pub close_pressed: Color32,
    pub on_close: Color32,
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
            nav: APP_COLOR,
            accent_text: APP_INK,
            on_accent: Color32::WHITE,
            input: Color32::WHITE,
            input_border: Color32::from_rgb(0x8A, 0x8A, 0x8A),
            group_label: Color32::from_rgb(0x70, 0x70, 0x70),
            ruler: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            ruler_margin: Color32::from_rgb(0xD4, 0xD4, 0xD4),
            ruler_tick: Color32::from_rgb(0x70, 0x70, 0x70),
            status_bar: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            selection: Color32::from_rgba_unmultiplied(0x3B, 0x5B, 0xDB, 0x48),
            find_highlight: Color32::from_rgba_unmultiplied(0xFF, 0xE0, 0x3D, 0x90),
            page_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x22),
            handle: Color32::WHITE,
            handle_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x28),
            guide: Color32::from_rgb(0xC0, 0x28, 0x8C),
            equation_box: Color32::from_rgb(0x6E, 0x9B, 0xE6),
            crop_handle: Color32::BLACK,
            crop_dim: Color32::from_rgba_unmultiplied(0x30, 0x30, 0x30, 0xE0),
            balloon: Color32::WHITE,
            markup_area: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            markup_edge: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            resolved_text: Color32::from_rgb(0x1B, 0x6B, 0x42),
            author_colors: [
                Color32::from_rgb(0x2F, 0x6F, 0xD0),
                Color32::from_rgb(0xB0, 0x3A, 0x2E),
                Color32::from_rgb(0x2E, 0x7D, 0x32),
                Color32::from_rgb(0x8E, 0x44, 0xAD),
                Color32::from_rgb(0xE0, 0x7B, 0x1F),
            ],
            leader_colors: [
                Color32::from_rgb(0x2F, 0x6F, 0xD0),
                Color32::from_rgb(0xB0, 0x3A, 0x2E),
                Color32::from_rgb(0x2E, 0x7D, 0x32),
                Color32::from_rgb(0x8E, 0x44, 0xAD),
                Color32::from_rgb(0xE0, 0x7B, 0x1F),
            ],
            menu: Color32::WHITE,
            blue: Color32::from_rgb(0x2F, 0x6F, 0xD0),
            red: Color32::from_rgb(0xD1, 0x3B, 0x3B),
            green: Color32::from_rgb(0x2E, 0x8B, 0x57),
            orange: Color32::from_rgb(0xE0, 0x7B, 0x1F),
            close_hover: Color32::from_rgb(0xC4, 0x2B, 0x1C),
            close_pressed: Color32::from_rgb(0xA6, 0x23, 0x16),
            on_close: Color32::WHITE,
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
            nav: Color32::from_rgb(0x27, 0x31, 0x5C),
            accent_text: Color32::from_rgb(0x9C, 0xB0, 0xF5),
            // Dark text on the light dark-theme accent: white on #7a93f0 is only 2.9:1 (#655).
            on_accent: Color32::from_rgb(0x11, 0x16, 0x2E),
            input: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            input_border: Color32::from_rgb(0x7A, 0x7A, 0x7A),
            group_label: Color32::from_rgb(0xA0, 0xA0, 0xA0),
            // The text area is lighter than the margins, as in the light theme (#1148).
            ruler: Color32::from_rgb(0x45, 0x45, 0x45),
            ruler_margin: Color32::from_rgb(0x2E, 0x2E, 0x2E),
            ruler_tick: Color32::from_rgb(0xB4, 0xB4, 0xB4),
            status_bar: Color32::from_rgb(0x1F, 0x1F, 0x1F),
            selection: Color32::from_rgba_unmultiplied(0x7A, 0x93, 0xF0, 0x55),
            find_highlight: Color32::from_rgba_unmultiplied(0xC8, 0xA8, 0x00, 0x70),
            page_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x60),
            handle: Color32::from_rgb(0xF2, 0xF2, 0xF2),
            handle_shadow: Color32::from_rgba_unmultiplied(0, 0, 0, 0x5A),
            guide: Color32::from_rgb(0xE0, 0x4C, 0xB0),
            equation_box: Color32::from_rgb(0x6E, 0x9B, 0xE6),
            crop_handle: Color32::BLACK,
            crop_dim: Color32::from_rgba_unmultiplied(0x30, 0x30, 0x30, 0xE0),
            balloon: Color32::from_rgb(0x2B, 0x2B, 0x2B),
            markup_area: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            markup_edge: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            resolved_text: Color32::from_rgb(0x5B, 0xC0, 0x8A),
            author_colors: [
                Color32::from_rgb(0x7F, 0xA8, 0xF2),
                Color32::from_rgb(0xE5, 0x78, 0x6B),
                Color32::from_rgb(0x6F, 0xBF, 0x73),
                Color32::from_rgb(0xC0, 0x8F, 0xE0),
                Color32::from_rgb(0xF0, 0xA0, 0x50),
            ],
            leader_colors: [
                Color32::from_rgb(0x2F, 0x6F, 0xD0),
                Color32::from_rgb(0xB0, 0x3A, 0x2E),
                Color32::from_rgb(0x2E, 0x7D, 0x32),
                Color32::from_rgb(0x8E, 0x44, 0xAD),
                Color32::from_rgb(0xE0, 0x7B, 0x1F),
            ],
            menu: Color32::from_rgb(0x2B, 0x2B, 0x2B),
            blue: Color32::from_rgb(0x6C, 0x9B, 0xEA),
            red: Color32::from_rgb(0xEE, 0x6A, 0x6A),
            green: Color32::from_rgb(0x5B, 0xC0, 0x8A),
            orange: Color32::from_rgb(0xF0, 0xA0, 0x50),
            // A little lighter than the light theme's red, so it reads against the dark window.
            close_hover: Color32::from_rgb(0xD6, 0x3C, 0x2C),
            close_pressed: Color32::from_rgb(0xB3, 0x30, 0x23),
            on_close: Color32::WHITE,
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

/// [`install_fonts`] with the Chinese or Japanese CJK fallback order. The new fonts apply from the
/// next frame.
pub fn install_fonts_for(ctx: &egui::Context, prefer_hans: bool) {
    install_fonts_with(ctx, prefer_hans, false);
}

/// [`install_fonts_for`] with the CJK fallback order for a Korean interface.
pub fn install_fonts_language(ctx: &egui::Context, prefer_hans: bool, prefer_korean: bool) {
    install_fonts_language_with(ctx, prefer_hans, prefer_korean, false, false);
}

/// [`install_fonts_for`], also adding an installed CJK font after the embedded faces when
/// `system_cjk` is set and the embedded faces leave CJK characters of the interface text without
/// a glyph (#241, #486). Reading that font is a large file read, so callers ask for it only when
/// CJK text is on screen: a CJK interface language, or the language names in Options.
pub fn install_fonts_with(ctx: &egui::Context, prefer_hans: bool, system_cjk: bool) {
    install_fonts_extra(ctx, prefer_hans, system_cjk, &[]);
}

/// [`install_fonts_with`] with the CJK font order selected for the interface language.
pub fn install_fonts_language_with(ctx: &egui::Context, prefer_hans: bool, prefer_korean: bool, prefer_hant: bool, system_cjk: bool) {
    install_fonts_language_extra(ctx, prefer_hans, prefer_korean, prefer_hant, system_cjk, &[]);
}

/// [`install_fonts_with`], also adding document-font `faces` as last-resort fallbacks (#565): the
/// faces that cover text the interface fonts cannot draw (an input method's composition, or a
/// file name in the window caption).
pub fn install_fonts_extra(ctx: &egui::Context, prefer_hans: bool, system_cjk: bool, faces: &[Arc<wordcraft_fonts::FontFace>]) {
    install_fonts_language_extra(ctx, prefer_hans, false, false, system_cjk, faces);
}

/// [`install_fonts_extra`] with the CJK font order selected for the interface language.
pub fn install_fonts_language_extra(
    ctx: &egui::Context,
    prefer_hans: bool,
    prefer_korean: bool,
    prefer_hant: bool,
    system_cjk: bool,
    faces: &[Arc<wordcraft_fonts::FontFace>],
) {
    let mut fonts = font_definitions_language(prefer_hans, prefer_korean, prefer_hant, system_cjk);
    add_fallback_faces(&mut fonts, faces);
    ctx.set_fonts(fonts);
}

/// Add `faces` after every other font of the interface families.
pub fn add_fallback_faces(fonts: &mut FontDefinitions, faces: &[Arc<wordcraft_fonts::FontFace>]) {
    for f in faces {
        let name = format!("fallback {} {} {}", f.family, f.style, f.id());
        let mut data = FontData::from_owned(f.data().to_vec());
        data.index = f.index();
        fonts.font_data.insert(name.clone(), Arc::new(data));
        for fam in [FontFamily::Proportional, FontFamily::Name("medium".into()), FontFamily::Name("semibold".into())] {
            if let Some(v) = fonts.families.get_mut(&fam) {
                v.push(name.clone());
            }
        }
    }
}

/// A character the window caption can already draw without a document fallback: the Latin range
/// the interface faces cover, or CJK, which uses the CJK UI font ([`install_fonts_with`]) rather
/// than a document face. System CJK collections are larger than [`faces_for_preedit`] will load.
pub fn caption_char_covered(c: char) -> bool {
    (c as u32) < 0x250 || wordcraft_fonts::is_cjk(c)
}

/// Document faces needed to draw `text` (the input method's composition, or a file name in the
/// window caption), one for each character the interface fonts cannot draw, skipping faces in
/// `have` (#565). Characters of the embedded interface fonts are not looked up; very large faces
/// (system CJK collections) are left to the CJK fallback.
pub fn faces_for_preedit(
    text: &str,
    have: &[Arc<wordcraft_fonts::FontFace>],
    covered: impl Fn(char) -> bool,
    lookup: impl Fn(char) -> Option<Arc<wordcraft_fonts::FontFace>>,
) -> Vec<Arc<wordcraft_fonts::FontFace>> {
    let mut out: Vec<Arc<wordcraft_fonts::FontFace>> = Vec::new();
    for c in text.chars().filter(|c| !c.is_ascii() && !c.is_whitespace() && !covered(*c)) {
        if have.iter().chain(&out).any(|f| f.covers(c)) {
            continue;
        }
        if let Some(f) = lookup(c).filter(|f| f.data().len() <= 16 << 20 && !have.iter().chain(&out).any(|h| h.id() == f.id())) {
            out.push(f);
        }
    }
    out
}

/// The interface fonts; see [`install_fonts_with`].
pub fn font_definitions(prefer_hans: bool, system_cjk: bool) -> FontDefinitions {
    font_definitions_language(prefer_hans, false, false, system_cjk)
}

/// The interface fonts, ordered for the preferred CJK writing system.
pub fn font_definitions_language(prefer_hans: bool, prefer_korean: bool, prefer_hant: bool, system_cjk: bool) -> FontDefinitions {
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
    #[cfg(not(target_arch = "wasm32"))]
    if prefer_hant
        && system_cjk
        && let Some(f) = system_hant_font()
    {
        let mut data = FontData::from_static(&f.bytes);
        data.index = f.index;
        add_cjk(&mut fonts, format!("system hant {}", f.family), data);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = prefer_hant;
    let cjk = wordcraft_fonts::ui_cjk_fonts_for(prefer_hans, prefer_korean);
    for f in &cjk {
        // The same static bytes the document fonts use: one copy in the binary.
        add_cjk(&mut fonts, format!("{} {}", f.family, f.style), FontData::from_static(f.bytes));
    }
    // Georgian has no glyphs in Inter, Source Sans or the CJK faces: the craft-fonts Geor face.
    for f in wordcraft_fonts::georgian_fonts() {
        add_cjk(&mut fonts, format!("{} {}", f.family, f.style), FontData::from_static(f.bytes));
    }
    // The embedded faces leave interface characters without a glyph (a build without craft-fonts,
    // without its Chinese face, #241, or with faces lacking catalog characters, #486): an
    // installed CJK font after them, so the menus don't show boxes or `?`.
    #[cfg(not(target_arch = "wasm32"))]
    if system_cjk && let Some(f) = system_cjk_font(prefer_hans, prefer_korean) {
        let mut data = FontData::from_static(&f.bytes);
        data.index = f.index;
        add_cjk(&mut fonts, format!("system {}", f.family), data);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = system_cjk;
    // Symbols and emoji fall back to egui's defaults (kept in the families).
    fonts
}

/// The installed CJK interface font for a Chinese (`hans`) or other interface, chosen once:
/// `None` when the embedded faces already cover the interface text
/// ([`cjk_font_text`](crate::i18n::Lang::cjk_font_text)). The font is picked for the catalogs'
/// missing characters, or for the language names when only those are missing.
#[cfg(not(target_arch = "wasm32"))]
fn system_cjk_font(hans: bool, korean: bool) -> Option<&'static wordcraft_fonts::SystemUiFont> {
    static ZH: std::sync::OnceLock<Option<wordcraft_fonts::SystemUiFont>> = std::sync::OnceLock::new();
    static OTHER: std::sync::OnceLock<Option<wordcraft_fonts::SystemUiFont>> = std::sync::OnceLock::new();
    static KO: std::sync::OnceLock<Option<wordcraft_fonts::SystemUiFont>> = std::sync::OnceLock::new();
    (if hans {
        &ZH
    } else if korean {
        &KO
    } else {
        &OTHER
    })
    .get_or_init(|| {
        let embedded = wordcraft_fonts::ui_cjk_fonts_for(hans, korean);
        let (catalogs, names) = crate::i18n::Lang::cjk_font_text_for(hans, korean);
        let mut needed = wordcraft_fonts::ui_uncovered_cjk(&catalogs, &embedded);
        if needed.is_empty() {
            needed = wordcraft_fonts::ui_uncovered_cjk(&[&names], &embedded);
            if needed.is_empty() {
                return None;
            }
        }
        log::debug!("interface: {} CJK characters lack an embedded glyph", needed.len());
        wordcraft_fonts::system_cjk_ui_font_for(hans, korean, &needed)
    })
    .as_ref()
}

#[cfg(not(target_arch = "wasm32"))]
fn system_hant_font() -> Option<&'static wordcraft_fonts::SystemUiFont> {
    static HANT: std::sync::OnceLock<Option<wordcraft_fonts::SystemUiFont>> = std::sync::OnceLock::new();
    HANT.get_or_init(|| {
        let f = wordcraft_fonts::system_hant_ui_font(&crate::i18n::Lang::hant_font_chars());
        log::debug!("interface: Traditional Chinese font {:?}", f.as_ref().map(|f| &f.family));
        f
    })
    .as_ref()
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

/// The text as it fits in `width`: shortened with an ellipsis when it is too long, and empty
/// when not even the ellipsis fits (#2402). The beginning of the text is kept, so the reader
/// still sees what the message is about.
pub fn fit_text(painter: &egui::Painter, text: &str, font: &FontId, tracking: f32, color: Color32, width: f32) -> String {
    if text.is_empty() || width <= 0.0 {
        return String::new();
    }
    let full = text_width(painter, text, font, tracking, color);
    if full <= width {
        return text.to_string();
    }
    let count = text.chars().count();
    // A rough cut first, so a long text doesn't need a layout for every character.
    let per_char = (full / count as f32).max(0.5);
    let mut shown: String = text.chars().take((width / per_char) as usize).collect();
    while !shown.is_empty() && text_width(painter, &format!("{shown}…"), font, tracking, color) > width {
        shown.pop();
    }
    if shown.is_empty() {
        // Only the ellipsis is left: it stands for the whole text when it fits.
        if text_width(painter, "…", font, tracking, color) <= width {
            return "…".to_string();
        }
        return String::new();
    }
    shown.push('…');
    shown
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

/// Apply the interface theme setting to egui. Both palettes go in, our light one as egui's light
/// style and our dark one as its dark style, and egui picks between them each frame: `System`
/// leaves egui's theme preference on `System`, so it follows the OS appearance it reports (light
/// when the OS reports none, like [`Appearance::is_dark`]) and the native window keeps its own
/// appearance; a manual Light or Dark pins egui (and the window decorations) to that theme.
/// Setting a concrete theme for `System` would pin the native window too, and macOS then stops
/// reporting appearance changes (#311).
pub fn apply(ctx: &egui::Context, appearance: Appearance) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(&Tokens::light()));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&Tokens::dark()));
    ctx.all_styles_mut(|s| {
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
    ctx.options_mut(|o| {
        o.fallback_theme = egui::Theme::Light;
        o.theme_preference = appearance.preference();
    });
}

/// Turn egui's own animations on or off: hover and press fades, menus, tooltips and windows fading
/// in, sections sliding open, and the glide when a scroll area brings the caret or a list item
/// into view. Off, each of them happens at once.
pub fn apply_animations(ctx: &egui::Context, on: bool) {
    let defaults = egui::Style::default();
    ctx.all_styles_mut(|s| {
        s.animation_time = if on { defaults.animation_time } else { 0.0 };
        s.scroll_animation = if on { defaults.scroll_animation } else { egui::style::ScrollAnimation::none() };
    });
}

/// egui's visuals for one of our palettes.
fn visuals(t: &Tokens) -> Visuals {
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
    v
}

pub fn c32(c: wordcraft_doc::Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    /// WCAG contrast ratio between two opaque colours.
    pub(super) fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
        let lum = |c: egui::Color32| {
            let ch = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// #1885: the ✕ on a window's close box reads on its hover and pressed reds in both themes.
    #[test]
    fn close_box_glyph_reads_on_its_reds() {
        for t in [super::Tokens::light(), super::Tokens::dark()] {
            for (name, bg) in [("close_hover", t.close_hover), ("close_pressed", t.close_pressed)] {
                let r = contrast(t.on_close, bg);
                assert!(r >= 4.5, "dark={}: on_close on {name} is {r:.2}:1", t.dark);
            }
        }
    }

    /// #655: text and knobs on filled accent controls (Share, avatar, AutoSave, keytips) reach 4.5:1
    /// on the accent and on its hover shade, in both themes.
    #[test]
    fn on_accent_reads_on_the_accent() {
        for t in [super::Tokens::light(), super::Tokens::dark()] {
            for (name, bg) in [("accent", t.accent), ("hover", t.accent_text)] {
                let r = contrast(t.on_accent, bg);
                assert!(r >= 4.5, "dark={}: on_accent on {name} is {r:.2}:1", t.dark);
            }
        }
    }

    /// White text and icons on the File page's side bar reach 4.5:1 in both themes.
    #[test]
    fn the_file_sidebar_reads_in_both_themes() {
        for t in [super::Tokens::light(), super::Tokens::dark()] {
            let r = contrast(egui::Color32::WHITE, t.nav);
            assert!(r >= 4.5, "dark={}: white on the sidebar is {r:.2}:1", t.dark);
        }
    }

    /// #657: text field and combo outlines reach 3:1 against the field and everything it sits on.
    #[test]
    fn input_borders_reach_three_to_one() {
        for t in [super::Tokens::light(), super::Tokens::dark()] {
            for (name, bg) in [("input", t.input), ("ribbon", t.ribbon), ("panel", t.panel), ("menu", t.menu), ("title bar", t.title_bar)] {
                let r = contrast(t.input_border, bg);
                assert!(r >= 3.0, "dark={}: input_border on {name} is {r:.2}:1", t.dark);
            }
        }
    }

    /// #1148: on both rulers the text area is lighter than the margins in both themes, as in the
    /// light theme, and the numbers drawn on it still reach 4.5:1.
    #[test]
    fn ruler_text_area_is_lighter_than_its_margins() {
        let lum = |c: egui::Color32| contrast(c, egui::Color32::BLACK);
        for t in [super::Tokens::light(), super::Tokens::dark()] {
            assert!(lum(t.ruler) > lum(t.ruler_margin), "dark={}: ruler text area is darker than its margins", t.dark);
            let r = contrast(t.ruler_tick, t.ruler);
            assert!(r >= 4.5, "dark={}: ruler numbers on the text area are {r:.2}:1", t.dark);
        }
    }

    /// #2045: comment and revision balloons were drawn on a hard-coded white card, so the light
    /// text of the dark theme was near-invisible. Every text colour a balloon uses reaches 4.5:1
    /// on the card, and the author stripes and borders 3:1 on the card and the markup area.
    #[test]
    fn comment_balloons_are_readable() {
        for t in [super::Tokens::light(), super::Tokens::dark()] {
            for (name, card) in [("card", t.balloon), ("selected card", t.checked)] {
                for (what, fg) in [("text", t.text), ("dim text", t.text_dim), ("resolved label", t.resolved_text)] {
                    let r = contrast(fg, card);
                    assert!(r >= 4.5, "dark={}: {what} on the {name} is {r:.2}:1", t.dark);
                }
            }
            // The light stripes are unchanged; only the dark theme's were unreadable.
            for (i, c) in t.author_colors.iter().enumerate().filter(|_| t.dark) {
                for (name, bg) in [("card", t.balloon), ("selected card", t.checked), ("markup area", t.markup_area)] {
                    let r = contrast(*c, bg);
                    assert!(r >= 3.0, "dark={}: author colour {i} on the {name} is {r:.2}:1", t.dark);
                }
            }
            // The leader's on-page part is drawn at 70% on white paper in both themes, no fainter than before (#2045).
            for (i, c) in t.leader_colors.iter().enumerate() {
                let on_paper = |a: u8| u8::try_from((u16::from(a) * 7 + 255 * 3) / 10).unwrap_or(255);
                let blended = egui::Color32::from_rgb(on_paper(c.r()), on_paper(c.g()), on_paper(c.b()));
                let r = contrast(blended, egui::Color32::WHITE);
                assert!(r >= 2.1, "dark={}: leader colour {i} on the page is {r:.2}:1", t.dark);
            }
            let r = contrast(t.markup_edge, t.markup_area);
            assert!(r >= 1.1, "dark={}: the markup area edge is invisible ({r:.2}:1)", t.dark);
        }
    }

    /// The test root declares egui's `default_fonts` and `monochrome_emoji_fonts` features
    /// explicitly (kittest's own dev-dependencies do not propagate). WordCraft inserts Inter at
    /// the front and SourceSans after egui's complete default fallback block, including the
    /// monochrome emoji faces. Production keeps only `default_fonts` through eframe.
    #[test]
    fn interface_font_order_keeps_test_default_fonts_explicit() {
        let defs = super::font_definitions(false, false);
        let prop = &defs.families[&egui::FontFamily::Proportional];
        assert_eq!(
            prop.iter().take(6).map(String::as_str).collect::<Vec<_>>(),
            ["Inter", "Ubuntu-Light", "egui-icons", "NotoEmoji-Regular", "emoji-icon-font", "SourceSans"]
        );
    }

    /// #565: the composition of an input method (Bangla through ibus) drew as boxes because only the
    /// interface fonts were used. Faces found for the uncovered characters are added as fallbacks,
    /// once, and not for characters the interface fonts draw.
    #[test]
    fn preedit_gets_document_fallback_faces() {
        let face = wordcraft_fonts::FontDb::global().face("Source Sans 3", "Regular");
        let lookup = |_: char| Some(face.clone());
        let bangla = "\u{0986}\u{09AE}\u{09BF}";
        let got = super::faces_for_preedit(bangla, &[], |_| false, lookup);
        assert_eq!(got.len(), 1, "one face, not one per character");
        assert!(super::faces_for_preedit(bangla, &got, |_| false, lookup).is_empty(), "already added");
        assert!(super::faces_for_preedit(bangla, &[], |_| true, lookup).is_empty(), "the interface fonts draw it");
        assert!(super::faces_for_preedit("abc", &[], |_| false, lookup).is_empty(), "ASCII never needs one");
        let mut defs = super::font_definitions(false, false);
        let before = defs.font_data.len();
        super::add_fallback_faces(&mut defs, &got);
        assert_eq!(defs.font_data.len(), before + 1);
        assert!(defs.families[&egui::FontFamily::Proportional].last().is_some_and(|n| n.starts_with("fallback ")));
    }

    /// A file name in a script Inter lacks (Hebrew, Arabic, …) is drawn in the window caption,
    /// which uses the semibold face. egui paints a missing glyph as `?`, so the caption borrows
    /// one document fallback face. CJK stays with the CJK UI font: those files are too large to
    /// add here.
    #[test]
    fn caption_borrows_one_fallback_face_for_hebrew() {
        let face = wordcraft_fonts::FontDb::global().face("Source Sans 3", "Regular");
        let lookup = |c: char| {
            assert!(!super::caption_char_covered(c), "{c} is already drawn by the interface fonts");
            Some(face.clone())
        };
        let hebrew = "מסמך";
        let got = super::faces_for_preedit(hebrew, &[], super::caption_char_covered, lookup);
        assert_eq!(got.len(), 1, "one face for the whole file name");
        assert!(super::faces_for_preedit(hebrew, &got, super::caption_char_covered, lookup).is_empty(), "already added");
        assert!(super::faces_for_preedit("文档", &[], super::caption_char_covered, lookup).is_empty(), "CJK uses the CJK UI font");
        assert!(super::faces_for_preedit("report", &[], super::caption_char_covered, lookup).is_empty(), "Latin needs nothing");
        let mut defs = super::font_definitions(false, false);
        super::add_fallback_faces(&mut defs, &got);
        let semi = &defs.families[&egui::FontFamily::Name("semibold".into())];
        assert!(semi.iter().any(|n| n.starts_with("fallback ")), "the caption is painted with semibold");
    }

    /// #656: dialog primary buttons, the Discord pill, the table eraser and the Navigation search
    /// highlight take their colours from the theme, not fixed light-theme values.
    #[test]
    fn interface_colours_come_from_the_theme() {
        let banned = ["fill(crate::theme::APP_COLOR)", "0x58, 0x65, 0xF2", "0x47, 0x52, 0xC4", "0xFF, 0xF1, 0x76", "0xD1, 0x3B, 0x3B"];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for path in crate::rust_sources(&dir) {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path == dir.join("theme.rs") || path == dir.join("icons.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for b in banned {
                assert!(!src.contains(b), "{name} hard-codes {b}; use theme::Tokens");
            }
        }
    }

    /// #2402: text that is too long for the room it has is shortened with an ellipsis, keeping
    /// its beginning; text that fits is left alone.
    #[test]
    fn fit_text_shortens_text_that_does_not_fit() {
        let long = "C:\\Test\\Documents\\Reports\\Final individual test folder\\Second level folder\\Project - Gadgets and sleep of teenagers.docx: The system cannot find the path specified. (os error 3)";
        let font = super::TypeRung::Caption.regular();
        let track = super::TypeRung::Caption.tracking();
        let ctx = egui::Context::default();
        let (mut whole, mut short, mut short_width, mut fitted) = (0.0, String::new(), 0.0, String::new());
        ctx.run_ui(egui::RawInput::default(), |ui| {
            whole = super::text_width(ui.painter(), long, &font, track, egui::Color32::BLACK);
            short = super::fit_text(ui.painter(), long, &font, track, egui::Color32::BLACK, whole / 2.0);
            short_width = super::text_width(ui.painter(), &short, &font, track, egui::Color32::BLACK);
            fitted = super::fit_text(ui.painter(), long, &font, track, egui::Color32::BLACK, whole);
        })
        .drop_without_applying_deltas();
        assert!(short.ends_with('…'), "{short}");
        assert!(long.starts_with(short.trim_end_matches('…')), "{short}");
        assert!(short_width <= whole / 2.0, "{short}");
        assert_eq!(fitted, long, "text that fits is left as it is");
    }

    /// Symbols written as text in the interface code must exist in the interface fonts (Inter and
    /// egui's emoji fallbacks); a missing glyph draws as an empty box, as the panes' "✕" close
    /// button once did. Letters are the catalogs' business (see the i18n font tests).
    #[test]
    fn interface_symbols_have_glyphs() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::font_definitions(false, false));
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut symbols = BTreeSet::new();
        // The i18n module's literals are catalog and pseudo-locale test text, not interface text.
        for path in crate::rust_sources(&dir).into_iter().filter(|p| !p.starts_with(dir.join("i18n"))) {
            let code = std::fs::read_to_string(&path).unwrap();
            // Characters inside string literals, line by line (good enough for this code base).
            for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for (i, part) in line.split('"').enumerate() {
                    if i % 2 == 1 {
                        symbols.extend(
                            part.chars()
                                .filter(|c| !c.is_ascii() && !c.is_alphabetic() && !c.is_whitespace())
                                .map(|c| (c, path.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/"))),
                        );
                    }
                }
            }
        }
        // Equation gallery templates are UnicodeMath input (`lim┬(n→∞)`, `≜`) that the math
        // renderer draws with document fonts, not interface text.
        // The ➢ bullet is a value for `para.bullets`; its menu tile draws the shape instead.
        let exempt = |c: char, file: &str| match file {
            "equation_tab.rs" => matches!(c, '≜' | '┬'),
            _ if file == "ribbon.rs" || file.starts_with("ribbon/") => c == '➢',
            _ => false,
        };
        symbols.retain(|(c, file)| !exempt(*c, file));
        let font = super::regular(12.5);
        let missing: Vec<String> = ctx.fonts_mut(|f| {
            symbols.iter().filter(|(c, _)| !f.has_glyph(&font, *c)).map(|(c, file)| format!("{c} U+{:04X} in {file}", *c as u32)).collect()
        });
        assert!(missing.is_empty(), "symbols without a glyph: {missing:#?}");
    }
}
