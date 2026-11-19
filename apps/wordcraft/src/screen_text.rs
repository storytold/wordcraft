//! Asks the system how it draws text, so page text follows the user's own setting
//! (`wordcraft_ui_egui::screen_text`, which says what each value means):
//!
//! - macOS: the `AppleFontSmoothing` default, read through `NSUserDefaults`.
//! - Windows: the font smoothing values under `HKCU\Control Panel\Desktop`, read from the
//!   registry.
//! - Linux and the BSDs: GNOME's and KDE Plasma's font settings, read through the XDG settings
//!   portal over D-Bus.
//!
//! All three are safe APIs (our workspace forbids unsafe). A thread asks at start-up and again
//! every few seconds, so a change in the system's settings shows without a restart and the UI
//! thread never waits for the system. Where the system says nothing, pages keep the platform's
//! own way of drawing text.

use std::time::Duration;

use wordcraft_ui_egui::screen_text::{self, ScreenText};

/// How often the system is asked again. Each question is a preferences lookup, a few registry
/// values or a handful of D-Bus calls.
const REFRESH: Duration = Duration::from_secs(3);

/// Start following the system's text settings. `ctx` is asked to redraw when they change how
/// pages are drawn.
pub fn watch(ctx: &egui::Context) {
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("screen-text-setting".into()).spawn(move || {
        let mut system = System::new();
        loop {
            if screen_text::set_system(system.read()) {
                ctx.request_repaint();
            }
            std::thread::sleep(REFRESH);
        }
    });
    if let Err(e) = spawned {
        log::warn!("screen text setting: {e}");
    }
}

/// The connection to whatever answers on this platform, kept between questions.
#[cfg(not(all(unix, not(target_os = "macos"))))]
struct System;

#[cfg(not(all(unix, not(target_os = "macos"))))]
impl System {
    fn new() -> Self {
        System
    }
}

#[cfg(target_os = "macos")]
impl System {
    /// `stringForKey:` gives a number as its text and nothing for a default that isn't set, which
    /// `integerForKey:` can't tell from 0 (smoothing turned off).
    fn read(&mut self) -> Option<ScreenText> {
        let level = objc2_foundation::NSUserDefaults::standardUserDefaults()
            .stringForKey(objc2_foundation::ns_string!("AppleFontSmoothing"))
            .and_then(|text| text.to_string().trim().parse::<i64>().ok());
        Some(ScreenText::from_macos(level))
    }
}

#[cfg(target_os = "windows")]
impl System {
    /// `None` when the key itself can't be opened; a value that is missing is left to
    /// [`ScreenText::from_windows`].
    fn read(&mut self) -> Option<ScreenText> {
        let desktop = windows_registry::CURRENT_USER.open(r"Control Panel\Desktop").map_err(|e| log::debug!("screen text setting: {e}")).ok()?;
        let smoothing = desktop.get_string("FontSmoothing").ok();
        Some(ScreenText::from_windows(
            smoothing.as_deref(),
            desktop.get_u32("FontSmoothingType").ok(),
            desktop.get_u32("FontSmoothingOrientation").ok(),
        ))
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
impl System {
    fn read(&mut self) -> Option<ScreenText> {
        None
    }
}

/// The session bus connection; opened by the first question, and again after one that failed.
#[cfg(all(unix, not(target_os = "macos")))]
struct System(Option<zbus::blocking::Connection>);

#[cfg(all(unix, not(target_os = "macos")))]
impl System {
    fn new() -> Self {
        System(None)
    }

    /// GNOME's settings where the portal has them, else KDE Plasma's; `None` where there is no
    /// session bus, no portal, or a desktop that keeps neither (the platform's light hinting
    /// stands).
    fn read(&mut self) -> Option<ScreenText> {
        const GNOME: &str = "org.gnome.desktop.interface";
        const KDE: &str = "org.kde.kdeglobals.General";
        if self.0.is_none() {
            self.0 = zbus::blocking::Connection::session().map_err(|e| log::debug!("screen text setting: no session bus ({e})")).ok();
        }
        let connection = self.0.as_ref()?;
        let mut answered = false;
        let mut ask = |namespace: &str, key: &str| -> Option<String> {
            let reply = connection
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.Settings"),
                    "ReadOne",
                    &(namespace, key),
                )
                .ok()?;
            answered = true;
            let (value,) = reply.body().deserialize::<(zbus::zvariant::OwnedValue,)>().ok()?;
            // Plasma hands a switch over as text or as a boolean, depending on its version.
            String::try_from(&*value).ok().or_else(|| bool::try_from(&*value).ok().map(|on| on.to_string()))
        };
        let gnome = [ask(GNOME, "font-antialiasing"), ask(GNOME, "font-hinting"), ask(GNOME, "font-rgba-order")];
        let kde = [ask(KDE, "XftAntialias"), ask(KDE, "XftHintStyle"), ask(KDE, "XftSubPixel")];
        if !answered {
            // Every call failed (an unknown setting is an error too): try a fresh connection
            // next time, in case the bus or the portal restarted.
            self.0 = None;
        }
        let [antialiasing, hinting, order] = gnome.each_ref().map(Option::as_deref);
        let [antialias, hint_style, sub_pixel] = kde.each_ref().map(Option::as_deref);
        ScreenText::from_gnome(antialiasing, hinting, order).or_else(|| ScreenText::from_kde(antialias, hint_style, sub_pixel))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::System;

    /// The real preferences answer, whatever this Mac is set to: one of the four strengths, and
    /// never hinting or subpixel edges, which macOS has no setting for.
    #[test]
    fn macos_answers_with_a_smoothing_strength() {
        let text = System::new().read().expect("macOS always answers");
        assert!([0.0, 0.5, 1.0, 1.5].contains(&text.darkening), "{text:?}");
        assert!(!text.hinting && text.subpixel.is_none(), "{text:?}");
    }
}
