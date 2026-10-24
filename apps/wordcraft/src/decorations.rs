//! Who draws the window frame on Linux and the BSDs.
//!
//! Wayland compositors that draw frames for their clients offer `zxdg_decoration_manager_v1`
//! (KDE, Sway, Hyprland, COSMIC). GNOME doesn't: it leaves the frame to the app, and winit then
//! drew an imitation of GNOME's above WordCraft's title bar. Without that protocol WordCraft
//! turns winit's frame off and its title bar becomes the header bar, as in GNOME's own apps
//! (`wordcraft_ui_egui::window_controls`), with the window buttons GNOME's `button-layout` names.
//! X11 window managers always draw the frame.
//!
//! `WORDCRAFT_TITLEBAR=app` or `=system` overrides the choice.

use wordcraft_ui_egui::window_controls::Layout;

/// The window buttons to draw in the app, or `None` when the desktop draws the frame. `x11`: the
/// app runs through X11 (XWayland included, `WORDCRAFT_X11`), whose window manager draws it.
pub(crate) fn app_drawn(x11: bool) -> Option<Layout> {
    if cfg!(any(target_os = "macos", target_os = "windows")) || x11 {
        return None;
    }
    let drawn = match std::env::var("WORDCRAFT_TITLEBAR").ok().as_deref().map(str::trim) {
        Some("app") => true,
        Some("system") => false,
        _ => compositor_leaves_frame_to_app(),
    };
    drawn.then(Layout::desktop)
}

/// Whether this is a Wayland session whose compositor draws no frames.
#[cfg(all(unix, not(target_os = "macos")))]
fn compositor_leaves_frame_to_app() -> bool {
    use wayland_client::Connection;
    use wayland_client::globals::registry_queue_init;
    // A launcher that hands over a connected socket (`WAYLAND_SOCKET`) hands over the only one:
    // connecting would take it, and winit would find no compositor. Keep the desktop's frame.
    if std::env::var_os("WAYLAND_SOCKET").is_some() {
        log::info!("WAYLAND_SOCKET is set; leaving the window frame to the desktop");
        return false;
    }
    // No Wayland compositor: X11, whose window manager draws the frame (winit falls back to it).
    let Ok(conn) = Connection::connect_to_env() else { return false };
    let Ok((globals, _queue)) = registry_queue_init::<Registry>(&conn) else { return false };
    let offered = globals.contents().with_list(|list| list.iter().any(|g| g.interface == "zxdg_decoration_manager_v1"));
    if !offered {
        log::info!("the Wayland compositor draws no window frames; WordCraft draws its own window buttons");
    }
    !offered
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn compositor_leaves_frame_to_app() -> bool {
    false
}

#[cfg(all(unix, not(target_os = "macos")))]
struct Registry;

#[cfg(all(unix, not(target_os = "macos")))]
impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for Registry {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    /// A launch through a handed-over `WAYLAND_SOCKET` keeps it for winit: the probe used to
    /// connect with it, which takes the variable, and the window then never opened. Run in a
    /// child process, since the probe reads (and used to change) the process environment.
    #[test]
    fn the_probe_leaves_a_handed_over_wayland_socket_to_winit() {
        if std::env::var_os("WORDCRAFT_TEST_PROBE_CHILD").is_some() {
            assert!(!super::compositor_leaves_frame_to_app());
            assert_eq!(std::env::var("WAYLAND_SOCKET").as_deref(), Ok("987"));
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "decorations::tests::the_probe_leaves_a_handed_over_wayland_socket_to_winit", "--nocapture"])
            .env("WORDCRAFT_TEST_PROBE_CHILD", "1")
            .env("WAYLAND_SOCKET", "987")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}
