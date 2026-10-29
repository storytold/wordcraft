//! The system's animation setting, which File › Options › Accessibility › Animation effects
//! follows when set to "Use system setting" (`Services::animations_enabled`).
//!
//! - Windows: Settings › Accessibility › Visual effects › Animation effects, read through the
//!   Windows Runtime's `UISettings`. Windows 7 has no Windows Runtime: the Windows 7 build
//!   (docs/windows7.md) doesn't ask, and the Options setting decides.
//! - macOS: System Settings › Accessibility › Display › Reduce motion, read through `NSWorkspace`.
//! - Linux and the BSDs: the desktop's setting, read through the settings portal ([`desktop`]).
//!
//! All three are safe APIs of crates already in the build (our workspace forbids unsafe). The UI
//! asks every frame, so the answer always comes from memory: Windows and macOS are asked again at
//! most once a second, and the Linux/BSD side keeps its answer up to date on a thread of its own.

#[cfg(any(all(unix, not(target_os = "macos")), test))]
mod desktop;

/// The answer to "does the system have animation effects on?" for the UI: `None` when the system
/// doesn't say. `ctx` is asked to redraw when an answer that arrives later changes it.
pub fn hook(ctx: &egui::Context) -> Box<dyn Fn() -> Option<bool>> {
    #[cfg(any(all(windows, not(target_vendor = "win7")), target_os = "macos"))]
    {
        let _ = ctx;
        Box::new(remembered(system_enabled, REFRESH))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Box::new(desktop::watch(ctx.clone()))
    }
    #[cfg(not(any(all(windows, not(target_vendor = "win7")), unix)))]
    {
        let _ = ctx;
        Box::new(|| None)
    }
}

/// How long an answer from Windows or macOS is kept before the system is asked again.
#[cfg(any(all(windows, not(target_vendor = "win7")), target_os = "macos"))]
const REFRESH: std::time::Duration = std::time::Duration::from_secs(1);

/// `read`, asked at most once per `every` and answered from memory in between.
#[cfg(any(all(windows, not(target_vendor = "win7")), target_os = "macos", test))]
fn remembered(read: impl Fn() -> Option<bool>, every: std::time::Duration) -> impl Fn() -> Option<bool> {
    let last = std::cell::Cell::new(None::<(std::time::Instant, Option<bool>)>);
    move || {
        if let Some((at, answer)) = last.get()
            && at.elapsed() < every
        {
            return answer;
        }
        let answer = read();
        last.set(Some((std::time::Instant::now(), answer)));
        answer
    }
}

/// Whether Windows has animation effects on; `None` when it can't be asked.
#[cfg(all(windows, not(target_vendor = "win7")))]
fn system_enabled() -> Option<bool> {
    windows::UI::ViewManagement::UISettings::new()
        .and_then(|settings| settings.AnimationsEnabled())
        .map_err(|e| log::debug!("system animation setting: {e}"))
        .ok()
}

/// Whether macOS plays animations: Reduce motion is off.
#[cfg(target_os = "macos")]
fn system_enabled() -> Option<bool> {
    Some(!objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use super::remembered;

    /// The UI asks every frame; the system is asked once per interval and the answer kept between.
    #[test]
    fn the_system_is_asked_once_per_interval() {
        let asked = Cell::new(0);
        let system = Cell::new(Some(false));
        let read = || {
            asked.set(asked.get() + 1);
            system.get()
        };
        let kept = remembered(read, Duration::from_secs(3600));
        assert_eq!((kept(), kept(), kept()), (Some(false), Some(false), Some(false)));
        system.set(Some(true));
        assert_eq!(kept(), Some(false), "within the interval the remembered answer stands");
        assert_eq!(asked.get(), 1);

        // Once the interval has passed (here: at once) the system is asked again.
        let fresh = remembered(read, Duration::ZERO);
        assert_eq!(fresh(), Some(true));
        system.set(None);
        assert_eq!(fresh(), None);
        assert_eq!(asked.get(), 3);
    }
}
