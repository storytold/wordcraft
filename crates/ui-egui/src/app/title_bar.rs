//! File › Options › General › Use system title bar (#78): where the desktop leaves the window frame
//! to the app (GNOME on Wayland), the framework's title bar carries the window buttons, in the
//! order of GNOME's `button-layout`; the setting gives the window the desktop's frame instead, at
//! once. craft-window decides the frame (`policy::decide_for`); the root window's frame is the one
//! further windows open with.

use craft::host::target::Backend;
use craft::ui::runtime::windows::WindowHost;
use craft::window::Prefix;
use craft::window::policy::{self, Decision, TitleBarPref};

use super::WordApp;

/// The prefix of the override variable, `WORDCRAFT_TITLEBAR` (`app`, or `system`).
pub(crate) const PREFIX: &str = "WORDCRAFT";

/// The window's frame for each value of Use system title bar, both decided at start-up from the
/// desktop as it was then, so the app's buttons come back in the desktop's `button-layout` order.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleBar {
    /// The frame with the setting off: the app's buttons.
    app: Decision,
    /// The frame with the setting on: the desktop's.
    system: Decision,
    /// The setting the window's frame follows now.
    applied: bool,
}

impl TitleBar {
    /// The frames for both values of the setting, or `None` to leave the frame to the desktop, as
    /// before #78. Only where the desktop draws no frame (a Wayland compositor without server-side
    /// decorations: GNOME) or `WORDCRAFT_TITLEBAR` asks: the policy would also draw the app's
    /// buttons on desktops that frame their windows (Windows, KDE), which is a decision for the
    /// whole suite rather than this setting.
    #[must_use]
    pub fn new(app: Decision, system: Decision, system_title_bar: bool) -> Option<Self> {
        let leaves_frame_to_app = app.desktop.ssd == Some(false);
        (leaves_frame_to_app || app.overridden.is_some()).then_some(Self { app, system, applied: system_title_bar })
    }

    /// The frame the window opens with.
    #[must_use]
    pub fn decision(&self) -> &Decision {
        if self.applied { &self.system } else { &self.app }
    }

    /// `Some(true)` when `WORDCRAFT_TITLEBAR` forced the app's buttons, `Some(false)` the desktop's
    /// frame; the setting then changes nothing.
    fn forced(&self) -> Option<bool> {
        self.app.overridden
    }
}

/// [`TitleBar::new`] for this desktop and `backend`.
#[must_use]
pub fn decide(backend: Backend, system_title_bar: bool) -> Option<TitleBar> {
    let decide = |pref| policy::decide_for(Prefix::new(PREFIX), backend, pref);
    TitleBar::new(decide(TitleBarPref::Integrated), decide(TitleBarPref::System), system_title_bar)
}

impl WordApp {
    /// The window opened with `title_bar`'s frame ([`decide`]): Use system title bar now changes it.
    #[must_use]
    pub fn with_title_bar(mut self, title_bar: TitleBar) -> Self {
        self.title_bar = Some(title_bar);
        self
    }

    /// Whether Use system title bar changes anything here: the framework decides the frame and
    /// `WORDCRAFT_TITLEBAR` doesn't.
    pub(crate) fn system_title_bar_applies(&self) -> bool {
        self.title_bar.as_ref().is_some_and(|t| t.forced().is_none())
    }

    /// Whether the app draws the window buttons (the result's `appDrawsFrame`).
    pub(crate) fn draws_window_frame(&self) -> bool {
        self.title_bar.as_ref().is_some_and(|t| t.forced().unwrap_or(!self.ui.system_title_bar))
    }

    /// Follow Use system title bar at once, from the window being drawn: the root window gets the
    /// setting's frame, and the other windows follow it.
    pub(crate) fn follow_title_bar(&mut self, ctx: &egui::Context) {
        let system = self.ui.system_title_bar;
        let Some(t) = self.title_bar.as_mut().filter(|t| t.forced().is_none() && t.applied != system) else { return };
        t.applied = system;
        WindowHost::of(ctx).set_root_frame(t.decision().clone());
    }
}
