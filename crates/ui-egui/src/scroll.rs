//! The user's scrolling-speed choice (issue #6146). The wheel and touchpad themselves are scrolled by the
//! framework's canvas navigation; this module keeps the setting and its multipliers.

/// The user's choice for scrolling speed, relative to the platform's normal speed (#6146).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollSpeed {
    /// Half the normal scroll speed.
    Slow,
    /// Normal scroll speed for the platform.
    #[default]
    Normal,
    /// Twice the normal scroll speed.
    Fast,
}

impl ScrollSpeed {
    /// All speed choices in presentation order: Slow, Normal, Fast.
    pub const ALL: [Self; 3] = [Self::Slow, Self::Normal, Self::Fast];

    /// The command / JSON code for this speed: `"slow"`, `"normal"`, or `"fast"`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Normal => "normal",
            Self::Fast => "fast",
        }
    }

    /// Parses a speed from a string code, case-insensitively.
    pub fn from_code(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "slow" => Some(Self::Slow),
            "normal" => Some(Self::Normal),
            "fast" => Some(Self::Fast),
            _ => None,
        }
    }

    /// User-facing label: `"Slow"`, `"Normal"`, or `"Fast"`.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Slow => "Slow",
            Self::Normal => "Normal",
            Self::Fast => "Fast",
        }
    }

    /// Speed multiplier relative to normal (1.0).
    pub const fn factor(self) -> f32 {
        match self {
            Self::Slow => 0.5,
            Self::Normal => 1.0,
            Self::Fast => 2.0,
        }
    }

    /// Points the view moves per point of touchpad travel.
    ///
    /// On Wayland (Linux/BSD), touchpads emit point deltas without compositor-side kinetic
    /// scaling or multiplier, which feels 4× slower than under XWayland / X11 (where winit emits
    /// fractional lines, ~1 line per 10 points, multiplied by line speed).
    /// On macOS and web browsers, the host platform already accelerates trackpad deltas and handles
    /// inertia, so Normal speed is 1.0.
    pub fn touchpad_factor(self) -> f32 {
        const NORMAL_TOUCHPAD_FACTOR: f32 = if cfg!(any(target_os = "macos", target_arch = "wasm32")) { 1.0 } else { 4.0 };
        NORMAL_TOUCHPAD_FACTOR * self.factor()
    }

    /// Multiplier for mouse-wheel notch distance.
    pub fn wheel_factor(self) -> f32 {
        self.factor()
    }
}

impl From<String> for ScrollSpeed {
    /// Parses a [`ScrollSpeed`] from a string, falling back to default.
    fn from(s: String) -> Self {
        ScrollSpeed::from_code(&s).unwrap_or_default()
    }
}

impl From<ScrollSpeed> for String {
    /// Converts a [`ScrollSpeed`] to its string representation.
    fn from(s: ScrollSpeed) -> Self {
        s.code().to_string()
    }
}
