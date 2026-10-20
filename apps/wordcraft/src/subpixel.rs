//! The desktop's subpixel text setting, which File › Options › Advanced › Subpixel text follows
//! when set to "Use system setting" (`Services::subpixel_order`).
//!
//! Linux and the BSDs, KDE Plasma and GNOME alike, two sources of the same setting:
//!
//! - the X resources the desktop's font settings write for X applications (`Xft.antialias`,
//!   `Xft.rgba`; KDE Plasma's System Settings › Text & Fonts › Sub-pixel rendering, GNOME's font
//!   antialiasing through its settings daemon), which fontconfig-based apps such as Firefox follow
//!   too, read from the root window's `RESOURCE_MANAGER` property;
//! - the settings portal: KDE Plasma's own keys (`org.kde.kdeglobals.General`: `XftAntialias`,
//!   `XftSubPixel`) first, then GNOME's (`org.gnome.desktop.interface`: `font-antialiasing`,
//!   `font-rgba-order`). A portal that merges both (KDE's with GTK's) answers GNOME's keys with
//!   GTK's defaults, not KDE's setting, so KDE's come first.
//!
//! Running through XWayland (the default) the X resources are asked first and the portal when
//! they say nothing; on native Wayland only the portal, so an on-demand XWayland (GNOME's, sway's
//! `xwayland lazy`) is never started to read a setting: where the portal has no font keys
//! (xdg-desktop-portal-wlr), page text stays greyscale unless File › Options says otherwise. A
//! thread asks every two seconds, so a change in the desktop's settings shows without a restart.
//! Elsewhere the desktop isn't asked: page text is greyscale.

use wordcraft_ui_egui::Subpixel;

/// The answer to "which subpixel order does the desktop draw text with?" for the UI: `None` for
/// greyscale text, or when it doesn't say. `ctx` is asked to redraw when the answer changes.
pub fn hook(ctx: &egui::Context) -> Box<dyn Fn() -> Option<Subpixel>> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Box::new(desktop::watch(ctx.clone()))
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = ctx;
        Box::new(|| None)
    }
}

/// What X resources (`RESOURCE_MANAGER`'s text, `name:\tvalue` lines) say: `Some(order)`, or
/// `Some(None)` for greyscale (antialiasing off, `Xft.rgba` `none`, or a vertical order, which
/// pages don't draw); `None` when they don't say (no `Xft.rgba`).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
pub fn resources_say(resources: &str) -> Option<Option<Subpixel>> {
    let value = |name: &str| {
        resources.lines().filter_map(|line| line.split_once(':')).find(|(k, _)| k.trim() == name).map(|(_, v)| v.trim().to_ascii_lowercase())
    };
    let rgba = value("Xft.rgba")?;
    if value("Xft.antialias").is_some_and(|a| a == "0" || a == "false") {
        return Some(None);
    }
    Some(order(&rgba))
}

/// What the settings portal's GNOME font keys say (`font-antialiasing`, `font-rgba-order`):
/// `None` when it doesn't have them.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
pub fn portal_says(antialiasing: Option<&str>, rgba_order: Option<&str>) -> Option<Option<Subpixel>> {
    match antialiasing?.trim().to_ascii_lowercase().as_str() {
        "rgba" => Some(rgba_order.and_then(order)),
        "grayscale" | "none" => Some(None),
        _ => None,
    }
}

/// What KDE Plasma's font settings say through the portal (`XftAntialias`, `XftSubPixel`):
/// `None` when it doesn't have them (not a KDE desktop).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
pub fn kde_says(antialias: Option<&str>, subpixel: Option<&str>) -> Option<Option<Subpixel>> {
    let subpixel = subpixel?;
    if antialias.is_some_and(|a| matches!(a.trim().to_ascii_lowercase().as_str(), "false" | "0")) {
        return Some(None);
    }
    Some(order(subpixel))
}

/// A fontconfig `rgba` name: `rgb` and `bgr` draw subpixel text; `none`, the vertical orders
/// (`vrgb`, `vbgr`) and anything else greyscale.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn order(name: &str) -> Option<Subpixel> {
    match name.trim().to_ascii_lowercase().as_str() {
        "rgb" => Some(Subpixel::Rgb),
        "bgr" => Some(Subpixel::Bgr),
        _ => None,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod desktop {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    use wordcraft_ui_egui::Subpixel;
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    /// How often the desktop is asked again.
    const EVERY: Duration = Duration::from_secs(2);
    /// Largest `RESOURCE_MANAGER` read, in 32-bit units (4 MB).
    const MAX_RESOURCES: u32 = 1 << 20;

    const GREYSCALE: u8 = 0;
    const RGB: u8 = 1;
    const BGR: u8 = 2;

    /// The latest answer, kept up to date on a thread of its own.
    pub fn watch(ctx: egui::Context) -> impl Fn() -> Option<Subpixel> {
        let latest = Arc::new(AtomicU8::new(GREYSCALE));
        let shared = latest.clone();
        // As the app itself chooses (`main`): XWayland when there's an X server, unless asked for
        // native Wayland.
        let native_wayland = std::env::var_os("DISPLAY").is_none() || std::env::var_os("WORDCRAFT_WAYLAND").is_some();
        let spawned = std::thread::Builder::new().name("subpixel-setting".into()).spawn(move || {
            // Both connections are opened when first needed and kept.
            let mut x: Option<(x11rb::rust_connection::RustConnection, usize)> = None;
            let mut bus: Option<Connection> = None;
            loop {
                let mut from_x = || {
                    if x.is_none() {
                        x = x11rb::connect(None).ok();
                    }
                    let answer = x.as_ref().and_then(|(conn, screen)| resources(conn, *screen)).and_then(|r| super::resources_say(&r));
                    // A connection the X server dropped is opened again next time.
                    if x.as_ref().is_some_and(|(conn, _)| conn.flush().is_err()) {
                        x = None;
                    }
                    answer
                };
                let mut from_portal = || {
                    if bus.is_none() {
                        bus = Connection::session().ok();
                    }
                    bus.as_ref().and_then(portal)
                };
                // On native Wayland only the portal: asking X there could start an on-demand
                // XWayland (GNOME's, sway's `xwayland lazy`) and keep it running.
                let answer = if native_wayland { from_portal() } else { from_x().or_else(from_portal) }.flatten();
                let code = match answer {
                    Some(Subpixel::Rgb) => RGB,
                    Some(Subpixel::Bgr) => BGR,
                    None => GREYSCALE,
                };
                if shared.swap(code, Ordering::Relaxed) != code {
                    ctx.request_repaint();
                }
                std::thread::sleep(EVERY);
            }
        });
        if let Err(e) = spawned {
            log::warn!("desktop subpixel setting: {e}");
        }
        move || match latest.load(Ordering::Relaxed) {
            RGB => Some(Subpixel::Rgb),
            BGR => Some(Subpixel::Bgr),
            _ => None,
        }
    }

    /// The root window's X resources.
    fn resources(conn: &impl x11rb::connection::Connection, screen: usize) -> Option<String> {
        let root = conn.setup().roots.get(screen)?.root;
        let reply = conn.get_property(false, root, AtomEnum::RESOURCE_MANAGER, AtomEnum::STRING, 0, MAX_RESOURCES).ok()?.reply().ok()?;
        String::from_utf8(reply.value).ok()
    }

    /// What the settings portal says: KDE Plasma's keys, else GNOME's; `None` when there's no
    /// portal or it has neither.
    fn portal(connection: &Connection) -> Option<Option<Subpixel>> {
        let portal =
            Proxy::new(connection, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings").ok()?;
        let read = |namespace: &str, key: &str| -> Option<String> {
            let value: OwnedValue = portal.call("ReadOne", &(namespace, key)).or_else(|_| portal.call("Read", &(namespace, key))).ok()?;
            text(&value, 0)
        };
        let kde = "org.kde.kdeglobals.General";
        let gnome = "org.gnome.desktop.interface";
        super::kde_says(read(kde, "XftAntialias").as_deref(), read(kde, "XftSubPixel").as_deref())
            .or_else(|| super::portal_says(read(gnome, "font-antialiasing").as_deref(), read(gnome, "font-rgba-order").as_deref()))
    }

    fn text(value: &Value<'_>, depth: u8) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.as_str().to_string()),
            // `Read` wraps its answer in a second variant.
            Value::Value(inner) if depth < 4 => text(inner, depth + 1),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_resources_give_the_order_kde_and_gnome_set() {
        // As KDE Plasma writes them (this machine: Sub-pixel rendering RGB).
        let kde = "Xft.antialias:\t1\nXft.dpi:\t139\nXft.hinting:\t1\nXft.hintstyle:\thintslight\nXft.rgba:\trgb\n";
        assert_eq!(resources_say(kde), Some(Some(Subpixel::Rgb)));
        assert_eq!(resources_say("Xft.rgba: BGR\n"), Some(Some(Subpixel::Bgr)));
        // Greyscale: no subpixel rendering, antialiasing off, or a vertical order.
        assert_eq!(resources_say("Xft.rgba:\tnone\n"), Some(None));
        assert_eq!(resources_say("Xft.antialias:\t0\nXft.rgba:\trgb\n"), Some(None));
        assert_eq!(resources_say("Xft.rgba:\tvrgb\n"), Some(None));
        // Nothing said: the portal is asked next.
        assert_eq!(resources_say("Xft.dpi:\t96\n*background: grey\n"), None);
        assert_eq!(resources_say(""), None);
        // Junk never panics.
        assert_eq!(resources_say(":::\n\u{0}Xft.rgba\n"), None);
    }

    #[test]
    fn the_portal_gives_kdes_font_settings_before_gnomes() {
        // As KDE Plasma's portal answers here (Sub-pixel rendering RGB, the default).
        assert_eq!(kde_says(Some("true"), Some("rgb")), Some(Some(Subpixel::Rgb)));
        assert_eq!(kde_says(None, Some("bgr")), Some(Some(Subpixel::Bgr)));
        assert_eq!(kde_says(Some("true"), Some("none")), Some(None));
        assert_eq!(kde_says(Some("true"), Some("vbgr")), Some(None));
        assert_eq!(kde_says(Some("false"), Some("rgb")), Some(None), "antialiasing off");
        // Not KDE: GNOME's keys are asked.
        assert_eq!(kde_says(None, None), None);
        assert_eq!(kde_says(Some("true"), None), None);
    }

    #[test]
    fn x_resources_as_gnomes_settings_daemon_writes_them() {
        // GNOME's default (font antialiasing greyscale), then Tweaks › Fonts › Subpixel.
        let grey = "Xft.antialias:\t1\nXft.hinting:\t1\nXft.hintstyle:\thintslight\nXft.rgba:\tnone\nXft.lcdfilter:\tlcddefault\nXft.dpi:\t96\n";
        assert_eq!(resources_say(grey), Some(None));
        let lcd = grey.replace("Xft.rgba:\tnone", "Xft.rgba:\trgb");
        assert_eq!(resources_say(&lcd), Some(Some(Subpixel::Rgb)));
    }

    #[test]
    fn the_portal_gives_gnomes_font_settings() {
        assert_eq!(portal_says(Some("rgba"), Some("rgb")), Some(Some(Subpixel::Rgb)));
        assert_eq!(portal_says(Some("rgba"), Some("bgr")), Some(Some(Subpixel::Bgr)));
        assert_eq!(portal_says(Some("rgba"), Some("vbgr")), Some(None));
        assert_eq!(portal_says(Some("rgba"), None), Some(None));
        assert_eq!(portal_says(Some("grayscale"), Some("rgb")), Some(None));
        assert_eq!(portal_says(Some("none"), None), Some(None));
        assert_eq!(portal_says(None, Some("rgb")), None);
        assert_eq!(portal_says(Some("sideways"), Some("rgb")), None);
    }
}
