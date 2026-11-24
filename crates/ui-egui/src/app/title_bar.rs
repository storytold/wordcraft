//! File › Options › General › Use system title bar (#78): where the desktop leaves the window frame
//! to the app (GNOME on Wayland), the framework's title bar carries the window buttons, in the
//! order of GNOME's `button-layout`; the setting gives the window the desktop's frame instead, at
//! once. craft-window decides the frame (`policy::decide_for`); the root window's frame is the one
//! further windows open with.

use craft::host::target::Backend;
use craft::ui::runtime::windows::WindowHost;
use craft::window::Prefix;
use craft::window::policy::{self, Decision, Desktop, Frame, TitleBarPref, WindowKind};

use super::WordApp;

/// The prefix of the override variable, `WORDCRAFT_TITLEBAR` (`app`, or `system`).
pub(crate) const PREFIX: &str = "WORDCRAFT";

/// How the window's frame was decided, kept so Use system title bar can decide it again.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleBar {
    /// The backend the window opened on: X11 when the app forced XWayland.
    backend: Backend,
    /// `Some(true)` when `WORDCRAFT_TITLEBAR` forced the app's buttons, `Some(false)` the desktop's
    /// frame; the setting then changes nothing.
    forced: Option<bool>,
    /// The desktop the frame was decided for, read once at start-up.
    desktop: Desktop,
    /// The setting the window's frame follows now.
    applied: bool,
}

/// The preference `system` stands for.
fn pref(system: bool) -> TitleBarPref {
    if system { TitleBarPref::System } else { TitleBarPref::Integrated }
}

/// The frame decision the desktop app hands the framework before the window opens, or `None` to
/// leave the frame to the desktop, as before #78. Only where the desktop draws no frame (a Wayland
/// compositor without server-side decorations: GNOME) or `WORDCRAFT_TITLEBAR` asks: the policy
/// would also draw the app's buttons on desktops that frame their windows (Windows, KDE), which
/// is a decision for the whole suite rather than this setting.
pub fn decide(backend: Backend, system_title_bar: bool) -> Option<Decision> {
    let decision = policy::decide_for(Prefix::new(PREFIX), backend, pref(system_title_bar));
    let leaves_frame_to_app = decision.desktop.ssd == Some(false);
    (leaves_frame_to_app || decision.overridden.is_some()).then_some(decision)
}

impl WordApp {
    /// The window opened with `decision`'s frame ([`decide`]), on `backend`: Use system title bar
    /// now changes it.
    #[must_use]
    pub fn with_title_bar(mut self, backend: Backend, decision: &Decision) -> Self {
        let applied = matches!(decision.frame, Frame::Native) && decision.overridden.is_none();
        self.title_bar = Some(TitleBar { backend, forced: decision.overridden, desktop: decision.desktop.clone(), applied });
        self
    }

    /// Whether Use system title bar changes anything here: the framework decides the frame and
    /// `WORDCRAFT_TITLEBAR` doesn't.
    pub(crate) fn system_title_bar_applies(&self) -> bool {
        self.title_bar.as_ref().is_some_and(|t| t.forced.is_none())
    }

    /// Whether the app draws the window buttons (the result's `appDrawsFrame`).
    pub(crate) fn draws_window_frame(&self) -> bool {
        self.title_bar.as_ref().is_some_and(|t| t.forced.unwrap_or(!self.ui.system_title_bar))
    }

    /// Follow Use system title bar at once, from the window being drawn: the root window gets the
    /// frame the policy decides for the setting, and the other windows follow it. The desktop is the
    /// one read at start-up; the framework keeps the buttons in the desktop's current order.
    pub(crate) fn follow_title_bar(&mut self, ctx: &egui::Context) {
        let system = self.ui.system_title_bar;
        let Some(t) = self.title_bar.as_mut().filter(|t| t.forced.is_none() && t.applied != system) else { return };
        t.applied = system;
        let decision = policy::decide(policy::Os::current(), policy::Runtime::current(), t.backend, &t.desktop, WindowKind::Main, pref(system), None);
        WindowHost::of(ctx).set_root_frame(decision);
    }
}
