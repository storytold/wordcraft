//! The interface settings saved in `ui.json`: theme, size and animation effects, and how each
//! maps onto the runtime's root values.

use craft::theme::{ModeChoice, MotionChoice};
use craft::ui::runtime::env::Scale;

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

    /// The runtime's mode choice: `System` stays the OS's answer rather than resolving here, so the
    /// native window keeps its own appearance.
    pub fn mode(self) -> ModeChoice {
        match self {
            Appearance::Light => ModeChoice::Light,
            Appearance::Dark => ModeChoice::Dark,
            Appearance::System => ModeChoice::System,
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
        self.drawn_percent_within(native, f32::INFINITY)
    }

    /// [`InterfaceSize::zoom_factor`] on a screen that [`auto_scale_limit`] caps at `limit` pixels
    /// per point: Auto never draws larger than that.
    pub fn zoom_factor_within(self, native: f32, limit: f32) -> f32 {
        match self.0 {
            None => (limit / usable_scale(native)).min(1.0),
            Some(_) => self.zoom_factor(native),
        }
    }

    /// The runtime's interface scale for this size: [`InterfaceSize::zoom_factor_within`] on a
    /// display of scale `native`, capped at `limit` under Auto.
    pub fn scale(self, native: f32, limit: f32) -> Scale {
        Scale::new(self.zoom_factor_within(native, limit))
    }

    /// [`InterfaceSize::drawn_percent`] within [`auto_scale_limit`]'s cap.
    pub fn drawn_percent_within(self, native: f32, limit: f32) -> u16 {
        let drawn = usable_scale(native) * self.zoom_factor_within(native, limit) * 100.0;
        drawn.round().clamp(0.0, f32::from(u16::MAX)) as u16
    }
}

/// The smallest window, in points, the interface is laid out for under Auto.
pub const AUTO_MIN_POINTS: (f32, f32) = (900.0, 460.0);

/// The largest pixels-per-point Auto may draw at on a screen of `screen_px` pixels (#4030): the
/// one that still leaves [`AUTO_MIN_POINTS`], so the ribbon and the window's controls stay on
/// screen. Never below 1.0, and unlimited when the screen size is unknown.
pub fn auto_scale_limit(screen_px: Option<(f32, f32)>) -> f32 {
    screen_px
        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0)
        .map_or(f32::INFINITY, |(w, h)| (w / AUTO_MIN_POINTS.0).min(h / AUTO_MIN_POINTS.1).max(1.0))
}

/// [`auto_scale_limit`] for the several windows that share one zoom, expressed as pixels per point
/// at the scale `native` (the one [`InterfaceSize::zoom_factor_within`] divides by). Each window's
/// monitor is its size in pixels and its own scale. Every window must stay within its own screen,
/// so the allowed zoom is worked out per monitor and the smallest one is shared. Unlimited when no
/// monitor is known.
pub fn auto_scale_limit_over(monitors: impl IntoIterator<Item = ([u32; 2], f32)>, native: f32) -> f32 {
    let shared_zoom =
        monitors.into_iter().map(|([w, h], scale)| auto_scale_limit(Some((w as f32, h as f32))) / usable_scale(scale)).fold(f32::INFINITY, f32::min);
    shared_zoom * usable_scale(native)
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
/// `System`, which follows the operating system's own setting as the runtime's motion policy
/// reads it. Off: wheel notches scroll and zoom at once and nothing fades or glides.
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

    /// The runtime's motion choice: `System` is the OS's reduced-motion answer, else full motion.
    pub fn motion(self) -> MotionChoice {
        match self {
            Animations::System => MotionChoice::System,
            Animations::On => MotionChoice::Full,
            Animations::Off => MotionChoice::Off,
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

// owed by ribbon batch 2 (S-L348): the base items, until their readers port. Local items above shadow them.
mod legacy;
#[allow(unused_imports)]
pub use legacy::*;
