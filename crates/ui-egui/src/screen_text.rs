//! How the desktop draws text, so pages look like the text in the platform's other apps.
//!
//! Each system keeps the user's choice somewhere else, and says different things:
//!
//! - macOS: the `AppleFontSmoothing` default, the strength of its font smoothing (0 to 3).
//! - Windows: `FontSmoothing`, `FontSmoothingType` and `FontSmoothingOrientation` under
//!   `HKCU\Control Panel\Desktop`: smoothing on or off, ClearType or greyscale, RGB or BGR stripes.
//! - Linux and the BSDs: the desktop's antialiasing, hinting and subpixel order, as GNOME
//!   (`org.gnome.desktop.interface`) or KDE Plasma (`kdeglobals`) keeps them.
//!
//! This module turns those values into one [`ScreenText`] and keeps the latest one for the page rasters to read.
//! It never asks the system itself, and for now nothing does: the app reads the system through craft-libs'
//! `platform::appearance`, which reports theme, contrast and reduced motion and no text settings yet. Until a
//! reader there hands an answer to [`set_system`] (and always in tests and headless screenshots)
//! [`ScreenText::PLATFORM`] stands, so pages are drawn each platform's fixed way. The constructors below are what
//! that reader's values go through.

use std::sync::Mutex;

/// The order of a screen's colour stripes, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubpixelOrder {
    Rgb,
    Bgr,
}

/// How page text is drawn on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenText {
    /// The strength of macOS-style font smoothing (glyphs thickened sideways): 0 is none, 1 the
    /// system's default. See `wordcraft_render::RenderOptions::text_darkening`.
    pub darkening: f32,
    /// Light hinting, as FreeType's `hintslight` draws text.
    pub hinting: bool,
    /// The system asks for subpixel (LCD) antialiasing, in this stripe order; `None` when it asks
    /// for greyscale antialiasing or says nothing.
    pub subpixel: Option<SubpixelOrder>,
    /// The screen's stripe order, where the system names one, whether or not it asks for
    /// subpixel antialiasing: a user who chooses subpixel text themselves gets it in the order
    /// of their screen. `None` when the system names no side-by-side order.
    pub stripes: Option<SubpixelOrder>,
}

impl ScreenText {
    /// What each platform's apps do when the user has changed nothing, and what pages are drawn
    /// with until the system has been asked: macOS smooths (thickens) text and never hints it;
    /// Linux and the BSDs hint lightly; Windows gets neither. All greyscale: a subpixel order is
    /// only ever taken from the system.
    pub const PLATFORM: ScreenText = ScreenText {
        darkening: if cfg!(target_os = "macos") { 1.0 } else { 0.0 },
        hinting: cfg!(all(unix, not(target_os = "macos"))),
        subpixel: None,
        stripes: None,
    };

    /// macOS, from the `AppleFontSmoothing` default (`None` when it isn't set). 0 turns the
    /// smoothing off; 1, 2 and 3 are light, medium and strong, and unset is medium. The three
    /// strengths as half, one and one and a half times our darkening are our own choice: Apple
    /// publishes no numbers for them. macOS has no hinting and no subpixel antialiasing to ask
    /// for (it dropped the latter in 10.14).
    pub fn from_macos(font_smoothing: Option<i64>) -> Self {
        let darkening = match font_smoothing {
            Some(level) if level <= 0 => 0.0,
            Some(1) => 0.5,
            Some(2) | None => 1.0,
            Some(_) => 1.5,
        };
        ScreenText { darkening, hinting: false, subpixel: None, stripes: None }
    }

    /// Windows, from the values under `HKCU\Control Panel\Desktop` (`None` for one that isn't
    /// there): `FontSmoothing` is the text "2" when edges are smoothed and "0" when not,
    /// `FontSmoothingType` is 2 for ClearType (1 is greyscale), `FontSmoothingOrientation` is 1
    /// for RGB stripes and 0 for BGR. ClearType gives subpixel edges, RGB unless the system
    /// says BGR; the stripe order itself is kept even with ClearType off. Text with smoothing off is still drawn antialiased, in greyscale: this
    /// rasteriser has no aliased text.
    pub fn from_windows(smoothing: Option<&str>, kind: Option<u32>, orientation: Option<u32>) -> Self {
        let smoothed = smoothing.map(str::trim) != Some("0");
        // The orientation is the screen's, and is kept whatever the smoothing is set to.
        let stripes = match orientation {
            Some(0) => Some(SubpixelOrder::Bgr),
            Some(1) => Some(SubpixelOrder::Rgb),
            _ => None,
        };
        let subpixel = (smoothed && kind == Some(2)).then_some(stripes.unwrap_or(SubpixelOrder::Rgb));
        ScreenText { darkening: 0.0, hinting: false, subpixel, stripes }
    }

    /// GNOME (and the desktops that keep its settings), from `org.gnome.desktop.interface`:
    /// `font-antialiasing` is `none`, `grayscale` or `rgba`; `font-hinting` is `none`, `slight`,
    /// `medium` or `full`; `font-rgba-order` is `rgba`, `rgb`, `bgr`, `vrgb` or `vbgr`. `None`
    /// when the desktop has none of the three. Any hinting but `none` is our one light hinting.
    /// Stripes that run across the screen (`vrgb`, `vbgr`) get greyscale edges: only side by
    /// side stripes are drawn.
    pub fn from_gnome(antialiasing: Option<&str>, hinting: Option<&str>, rgba_order: Option<&str>) -> Option<Self> {
        if antialiasing.is_none() && hinting.is_none() && rgba_order.is_none() {
            return None;
        }
        let order = match rgba_order.map(str::trim) {
            Some("bgr") => Some(SubpixelOrder::Bgr),
            Some("vrgb" | "vbgr") => None,
            // `rgba` is the schema's first value and its default; it and no answer mean RGB.
            _ => Some(SubpixelOrder::Rgb),
        };
        let subpixel = if antialiasing.map(str::trim) == Some("rgba") { order } else { None };
        // The order the desktop names, kept when it asks for greyscale.
        let stripes = order.filter(|_| rgba_order.is_some());
        Some(ScreenText { darkening: 0.0, hinting: hinting.map(str::trim) != Some("none"), subpixel, stripes })
    }

    /// KDE Plasma, from the `[General]` group of `kdeglobals`: `XftAntialias` is `true` or
    /// `false`; `XftHintStyle` is `hintnone`, `hintslight`, `hintmedium` or `hintfull`;
    /// `XftSubPixel` is `none`, `rgb`, `bgr`, `vrgb` or `vbgr`. `None` when it has none of the
    /// three (Plasma writes them only once the user has changed the font settings).
    pub fn from_kde(antialias: Option<&str>, hint_style: Option<&str>, sub_pixel: Option<&str>) -> Option<Self> {
        if antialias.is_none() && hint_style.is_none() && sub_pixel.is_none() {
            return None;
        }
        let antialiased = !matches!(antialias.map(|v| v.trim().to_ascii_lowercase()).as_deref(), Some("false" | "0"));
        let order = match sub_pixel.map(str::trim) {
            Some("rgb") => Some(SubpixelOrder::Rgb),
            Some("bgr") => Some(SubpixelOrder::Bgr),
            _ => None,
        };
        Some(ScreenText {
            darkening: 0.0,
            hinting: hint_style.map(str::trim) != Some("hintnone"),
            subpixel: order.filter(|_| antialiased),
            stripes: order,
        })
    }

    /// The same settings with numbers a raster can take: a darkening that isn't a number, or is
    /// negative, is none, and the strongest is twice the default.
    pub fn sanitized(self) -> Self {
        let darkening = if self.darkening.is_finite() { self.darkening.clamp(0.0, 2.0) } else { 0.0 };
        ScreenText { darkening, ..self }
    }

    /// Put these settings into a page raster's options.
    pub fn apply(self, opts: &mut wordcraft_render::RenderOptions) {
        let text = self.sanitized();
        opts.text_darkening = text.darkening;
        opts.hinting = text.hinting;
    }

    /// A value to hash into a page's cache key: a page drawn under other settings is another
    /// raster.
    pub fn key(self) -> (u32, bool, Option<SubpixelOrder>, Option<SubpixelOrder>) {
        let text = self.sanitized();
        (text.darkening.to_bits(), text.hinting, text.subpixel, text.stripes)
    }
}

/// What the system said last; `None` until the app has asked, and when the system said nothing.
static SYSTEM: Mutex<Option<ScreenText>> = Mutex::new(None);

/// How page text is drawn now: the system's settings once they are known ([`set_system`]), else
/// [`ScreenText::PLATFORM`].
pub fn current() -> ScreenText {
    SYSTEM.lock().unwrap_or_else(std::sync::PoisonError::into_inner).unwrap_or(ScreenText::PLATFORM)
}

/// Record the system's answer (`None`: it has none, so the platform's own way stands). Returns
/// whether that changes how text is drawn, so the caller can ask for a repaint.
pub fn set_system(answer: Option<ScreenText>) -> bool {
    let answer = answer.map(ScreenText::sanitized);
    let before = current();
    *SYSTEM.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = answer;
    current() != before
}

#[cfg(test)]
mod tests {
    use super::{ScreenText, SubpixelOrder};

    #[test]
    fn the_platform_default_is_what_was_built_in() {
        let p = ScreenText::PLATFORM;
        assert_eq!(p.darkening > 0.0, cfg!(target_os = "macos"));
        assert_eq!(p.hinting, cfg!(all(unix, not(target_os = "macos"))));
        assert_eq!((p.subpixel, p.stripes), (None, None), "a stripe order only ever comes from the system");
        // Tests and headless screenshots never ask the system, so this is what they draw with.
        assert_eq!(super::current(), p);
    }

    #[test]
    fn macos_font_smoothing_sets_the_darkening() {
        let strength = |level| ScreenText::from_macos(level).darkening;
        assert_eq!(strength(None), 1.0, "unset is the system's medium");
        assert_eq!((strength(Some(0)), strength(Some(1)), strength(Some(2)), strength(Some(3))), (0.0, 0.5, 1.0, 1.5));
        // Values Apple doesn't define: below the range is off, above it the strongest.
        assert_eq!((strength(Some(-4)), strength(Some(i64::MIN)), strength(Some(9)), strength(Some(i64::MAX))), (0.0, 0.0, 1.5, 1.5));
        let text = ScreenText::from_macos(None);
        assert!(!text.hinting && text.subpixel.is_none());
    }

    #[test]
    fn windows_cleartype_gives_subpixel_edges_in_the_screens_order() {
        let order = |smoothing, kind, orientation| ScreenText::from_windows(smoothing, kind, orientation).subpixel;
        assert_eq!(order(Some("2"), Some(2), Some(1)), Some(SubpixelOrder::Rgb));
        assert_eq!(order(Some("2"), Some(2), Some(0)), Some(SubpixelOrder::Bgr));
        assert_eq!(order(Some("2"), Some(2), None), Some(SubpixelOrder::Rgb), "no orientation: RGB, as nearly every panel is");
        assert_eq!(order(None, Some(2), Some(1)), Some(SubpixelOrder::Rgb), "no on/off value: on, as Windows starts");
        // Greyscale smoothing, smoothing off, and a machine that says nothing.
        assert_eq!(order(Some("2"), Some(1), Some(1)), None);
        assert_eq!(order(Some("0"), Some(2), Some(1)), None);
        assert_eq!(order(Some(" 0 "), Some(2), Some(1)), None);
        assert_eq!(order(None, None, None), None);
        assert_eq!(order(Some("junk"), Some(u32::MAX), Some(u32::MAX)), None);
        let text = ScreenText::from_windows(Some("2"), Some(2), Some(1));
        assert!(!text.hinting && text.darkening == 0.0);
        // The screen's stripe order survives ClearType being off or greyscale, for a user who
        // chooses subpixel text themselves; no orientation value is no known order.
        let stripes = |smoothing, kind, orientation| ScreenText::from_windows(smoothing, kind, orientation).stripes;
        assert_eq!(stripes(Some("0"), Some(2), Some(0)), Some(SubpixelOrder::Bgr));
        assert_eq!(stripes(Some("2"), Some(1), Some(0)), Some(SubpixelOrder::Bgr));
        assert_eq!(stripes(Some("0"), None, Some(1)), Some(SubpixelOrder::Rgb));
        assert_eq!(stripes(Some("2"), Some(2), None), None);
        assert_eq!(stripes(Some("2"), Some(2), Some(7)), None);
    }

    #[test]
    fn gnome_settings_set_hinting_and_subpixel_edges() {
        assert_eq!(ScreenText::from_gnome(None, None, None), None, "a desktop without these settings says nothing");
        let text = |a, h, o| ScreenText::from_gnome(a, h, o).unwrap();
        // GNOME's own defaults: greyscale, slight hinting.
        assert_eq!(
            text(Some("grayscale"), Some("slight"), Some("rgb")),
            ScreenText { darkening: 0.0, hinting: true, subpixel: None, stripes: Some(SubpixelOrder::Rgb) }
        );
        assert_eq!(text(Some("rgba"), Some("slight"), Some("rgb")).subpixel, Some(SubpixelOrder::Rgb));
        assert_eq!(text(Some("rgba"), Some("slight"), Some("rgba")).subpixel, Some(SubpixelOrder::Rgb));
        assert_eq!(text(Some("rgba"), Some("slight"), Some("bgr")).subpixel, Some(SubpixelOrder::Bgr));
        assert_eq!(text(Some("rgba"), None, None).subpixel, Some(SubpixelOrder::Rgb));
        // Stripes across the screen, and no antialiasing: greyscale edges.
        assert_eq!(text(Some("rgba"), Some("slight"), Some("vrgb")).subpixel, None);
        assert_eq!(text(Some("rgba"), Some("slight"), Some("vbgr")).subpixel, None);
        assert_eq!(text(Some("none"), Some("slight"), Some("rgb")).subpixel, None);
        // Hinting: off only when the desktop says none.
        assert!(!text(Some("grayscale"), Some("none"), None).hinting);
        for style in ["slight", "medium", "full", "something new"] {
            assert!(text(None, Some(style), None).hinting, "{style}");
        }
        assert!(text(Some("grayscale"), None, None).hinting);
        // The stripe order is kept when the desktop asks for greyscale or no antialiasing.
        assert_eq!(text(Some("grayscale"), None, Some("bgr")).stripes, Some(SubpixelOrder::Bgr));
        assert_eq!(text(Some("none"), None, Some("rgb")).stripes, Some(SubpixelOrder::Rgb));
        assert_eq!(text(Some("rgba"), None, None).stripes, None, "no order named");
        assert_eq!(text(Some("grayscale"), None, Some("vbgr")).stripes, None);
    }

    #[test]
    fn kde_settings_set_hinting_and_subpixel_edges() {
        assert_eq!(ScreenText::from_kde(None, None, None), None, "Plasma writes nothing until the font settings change");
        let text = |a, h, s| ScreenText::from_kde(a, h, s).unwrap();
        assert_eq!(
            text(Some("true"), Some("hintslight"), Some("rgb")),
            ScreenText { darkening: 0.0, hinting: true, subpixel: Some(SubpixelOrder::Rgb), stripes: Some(SubpixelOrder::Rgb) }
        );
        assert_eq!(text(Some("true"), Some("hintslight"), Some("bgr")).subpixel, Some(SubpixelOrder::Bgr));
        assert_eq!(text(None, None, Some("rgb")).subpixel, Some(SubpixelOrder::Rgb));
        for none in ["none", "vrgb", "vbgr", "junk"] {
            assert_eq!(text(Some("true"), None, Some(none)).subpixel, None, "{none}");
        }
        assert_eq!(text(Some("false"), None, Some("rgb")).subpixel, None, "antialiasing off");
        assert_eq!(text(Some(" False "), None, Some("rgb")).subpixel, None);
        assert!(!text(None, Some("hintnone"), None).hinting);
        assert!(text(None, Some("hintfull"), None).hinting && text(Some("true"), None, None).hinting);
        // The stripe order is kept with antialiasing off.
        assert_eq!(text(Some("false"), None, Some("bgr")).stripes, Some(SubpixelOrder::Bgr));
        assert_eq!(text(Some("true"), None, Some("none")).stripes, None);
    }

    #[test]
    fn settings_go_into_the_page_options_as_numbers_a_raster_can_take() {
        let mut opts = wordcraft_render::RenderOptions::default();
        ScreenText { darkening: 1.5, hinting: true, subpixel: None, stripes: None }.apply(&mut opts);
        assert_eq!((opts.text_darkening, opts.hinting), (1.5, true));
        for (given, taken) in [(f32::NAN, 0.0), (f32::INFINITY, 0.0), (-3.0, 0.0), (40.0, 2.0), (0.5, 0.5)] {
            let text = ScreenText { darkening: given, hinting: false, subpixel: None, stripes: None };
            text.apply(&mut opts);
            assert_eq!((opts.text_darkening, opts.hinting), (taken, false), "{given}");
            assert_eq!(text.key(), ScreenText { darkening: taken, ..text }.key());
        }
        // Each setting changes the cache key.
        let base = ScreenText { darkening: 1.0, hinting: false, subpixel: None, stripes: None };
        assert_ne!(base.key(), ScreenText { darkening: 0.5, ..base }.key());
        assert_ne!(base.key(), ScreenText { hinting: true, ..base }.key());
        assert_ne!(base.key(), ScreenText { subpixel: Some(SubpixelOrder::Rgb), ..base }.key());
        assert_ne!(base.key(), ScreenText { stripes: Some(SubpixelOrder::Bgr), ..base }.key());
        assert_ne!(ScreenText { subpixel: Some(SubpixelOrder::Bgr), ..base }.key(), ScreenText { subpixel: Some(SubpixelOrder::Rgb), ..base }.key());
    }
}
