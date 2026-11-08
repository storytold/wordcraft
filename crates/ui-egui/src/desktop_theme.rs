//! The desktop's own light/dark choice, for an interface setting that follows it.
//!
//! winit reports no system theme on Linux — `ActiveEventLoop::system_theme` is `None` on every
//! desktop there — so "Use system setting" has to ask the desktop itself. The XDG Desktop Portal's
//! Settings interface answers on GNOME, KDE and XFCE alike, over X11 and Wayland alike.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// How often [`DesktopTheme::start`] re-reads the desktop's answer. The portal also publishes a
/// change signal, but subscribing needs a connection kept alive for the life of every window; one
/// round trip every few seconds is cheap and reacts soon enough after a theme switch.
const POLL: Duration = Duration::from_secs(4);

/// Whether the desktop asks applications to draw dark. `None` when it says nothing, and when
/// there is no session bus or no portal to ask (a bare X server, a container), so the caller
/// keeps whatever it chose itself.
pub fn prefers_dark() -> Option<bool> {
    read(&mut Bus::new())
}

/// Watches the desktop and reports its choice when it changes. [`prefers_dark`] covers the
/// answer at start-up; this covers the user switching theme while the app runs.
#[derive(Default)]
pub struct DesktopTheme {
    /// `None` when no watcher thread could be started; the one-shot read still works.
    changes: Option<Receiver<Option<bool>>>,
    /// Whether the watcher thread has delivered at least one response.
    received: bool,
}

impl DesktopTheme {
    /// Start watching. Nothing runs before this is called, and calling it once is enough. The
    /// first answer is sent straight away, so the caller does not have to ask for it separately.
    /// `changed` runs on the watcher's thread after each answer is sent (the app asks for a
    /// repaint there, so an idle window picks the change up). The app binary starts this; tests
    /// and headless screenshots don't, so they never depend on the machine's desktop theme.
    pub fn start(changed: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let started = std::thread::Builder::new().name("wordcraft-desktop-theme".into()).spawn(move || {
            // One session-bus connection for the life of the watcher, opened again only after
            // it fails.
            let mut bus = Bus::new();
            let mut last = read(&mut bus);
            if tx.send(last).is_err() {
                return;
            }
            changed();
            loop {
                std::thread::sleep(POLL);
                let now = read(&mut bus);
                if now != last {
                    last = now;
                    if tx.send(now).is_err() {
                        return;
                    }
                    changed();
                }
            }
        });
        Self { changes: started.ok().map(|_| rx), received: false }
    }

    /// Whether a watcher thread is running.
    pub fn watching(&self) -> bool {
        self.changes.is_some()
    }

    /// Whether the watcher has delivered at least one response.
    pub fn received(&self) -> bool {
        self.received
    }

    /// The desktop's new answer if it changed since the last call, to be polled once a frame.
    /// Several changes in one frame collapse to the newest, and a desktop that stops answering
    /// leaves the last answer in place.
    pub fn take_change(&mut self) -> Option<Option<bool>> {
        let Some(changes) = &self.changes else { return None };
        let mut newest = None;
        let mut got_any = false;
        while let Ok(changed) = changes.try_recv() {
            newest = changed;
            got_any = true;
            self.received = true;
        }
        if got_any { Some(newest) } else { None }
    }
}

/// The watcher's session-bus connection, kept between polls.
#[cfg(target_os = "linux")]
struct Bus(Option<zbus::blocking::Connection>);

#[cfg(target_os = "linux")]
impl Bus {
    /// No connection yet: the first read opens one.
    fn new() -> Self {
        Bus(None)
    }
}

/// Elsewhere there is no session bus to keep.
#[cfg(not(target_os = "linux"))]
struct Bus;

#[cfg(not(target_os = "linux"))]
impl Bus {
    fn new() -> Self {
        Bus
    }
}

/// Ask the portal once, over `bus`'s connection (opened when there is none). D-Bus calls
/// block, so this belongs off the UI thread. A connection whose calls all fail is dropped, so
/// the next poll opens a fresh one (the bus or portal may have restarted).
#[cfg(target_os = "linux")]
fn read(bus: &mut Bus) -> Option<bool> {
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";
    const SCHEMAS: [(&str, &str); 3] = [
        ("org.freedesktop.appearance", "color-scheme"),
        ("org.gnome.desktop.interface", "color-scheme"),
        ("org.freedesktop.interface.color-scheme", "color-scheme"),
    ];
    if bus.0.is_none() {
        bus.0 = zbus::blocking::Connection::session().ok();
    }
    let connection = bus.0.as_ref()?;
    let mut answered = false;
    for (schema, key) in SCHEMAS {
        let reply = connection
            .call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(SETTINGS), "ReadOne", &(schema, key))
            .or_else(|_| connection.call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(SETTINGS), "Read", &(schema, key)));
        let Ok(reply) = reply else {
            continue;
        };
        answered = true;
        let Ok((answer,)) = reply.body().deserialize::<(zbus::zvariant::OwnedValue,)>() else { continue };
        if let Some(dark) = parse_value(&answer) {
            return Some(dark);
        }
    }
    if !answered {
        bus.0 = None;
    }
    None
}

/// Parse a variant value from the portal settings, which can be a u32 (1: dark, 2: light, 0: none)
/// per the XDG Desktop Portal specification, or a string ("prefer-dark", "prefer-light").
#[cfg(target_os = "linux")]
fn parse_value(val: &zbus::zvariant::Value) -> Option<bool> {
    match val {
        zbus::zvariant::Value::Value(inner) => parse_value(inner),
        zbus::zvariant::Value::U8(1)
        | zbus::zvariant::Value::U16(1)
        | zbus::zvariant::Value::U32(1)
        | zbus::zvariant::Value::U64(1)
        | zbus::zvariant::Value::I16(1)
        | zbus::zvariant::Value::I32(1)
        | zbus::zvariant::Value::I64(1) => Some(true),
        zbus::zvariant::Value::U8(2)
        | zbus::zvariant::Value::U16(2)
        | zbus::zvariant::Value::U32(2)
        | zbus::zvariant::Value::U64(2)
        | zbus::zvariant::Value::I16(2)
        | zbus::zvariant::Value::I32(2)
        | zbus::zvariant::Value::I64(2) => Some(false),
        zbus::zvariant::Value::Str(s) => parse(s.as_str()),
        _ => None,
    }
}

/// The portal's `color-scheme` answer. `default` means the desktop states no preference, so
/// neither does the application.
#[cfg(target_os = "linux")]
fn parse(scheme: &str) -> Option<bool> {
    let s = scheme.trim();
    if s.eq_ignore_ascii_case("prefer-dark") || s.eq_ignore_ascii_case("dark") || s == "1" {
        Some(true)
    } else if s.eq_ignore_ascii_case("prefer-light") || s.eq_ignore_ascii_case("light") || s == "2" {
        Some(false)
    } else {
        None
    }
}

/// macOS and Windows answer through winit, which the caller already has.
#[cfg(not(target_os = "linux"))]
fn read(_bus: &mut Bus) -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal answers with one of three strings; only the two preferences are a choice.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_portal_answer_reads_as_a_choice_or_as_none() {
        assert_eq!(parse("prefer-dark"), Some(true));
        assert_eq!(parse("prefer-light"), Some(false));
        assert_eq!(parse("default"), None);
        assert_eq!(parse(""), None);
    }

    /// The portal specification returns a u32 (1: dark, 2: light, 0: none) in a variant.
    #[test]
    #[cfg(target_os = "linux")]
    fn portal_value_parsing_handles_u32_and_variants() {
        use zbus::zvariant::Value;
        assert_eq!(parse_value(&Value::U32(1)), Some(true));
        assert_eq!(parse_value(&Value::U32(2)), Some(false));
        assert_eq!(parse_value(&Value::U32(0)), None);
        assert_eq!(parse_value(&Value::Value(Box::new(Value::U32(1)))), Some(true));
        assert_eq!(parse_value(&Value::Value(Box::new(Value::U32(2)))), Some(false));
        assert_eq!(parse_value(&Value::Str("prefer-dark".into())), Some(true));
        assert_eq!(parse_value(&Value::Str("prefer-light".into())), Some(false));
    }

    /// Off Linux the caller already has winit's answer, so this adds nothing.
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn other_platforms_answer_through_winit() {
        assert_eq!(prefers_dark(), None);
    }

    /// A watcher that was never started is inert rather than a panic.
    #[test]
    fn a_watcher_without_a_thread_is_inert() {
        assert_eq!(DesktopTheme::default().take_change(), None);
    }

    /// When the watcher thread delivers answers, `take_change` yields them and `received` turns true.
    #[test]
    fn take_change_delivers_answers_and_records_received() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut dt = DesktopTheme { changes: Some(rx), received: false };
        assert!(!dt.received());
        assert_eq!(dt.take_change(), None);
        assert!(!dt.received());

        // Send a dark preference.
        tx.send(Some(true)).unwrap();
        assert_eq!(dt.take_change(), Some(Some(true)));
        assert!(dt.received());

        // Empty channel leaves received true and returns None.
        assert_eq!(dt.take_change(), None);
        assert!(dt.received());

        // Send a light preference.
        tx.send(Some(false)).unwrap();
        assert_eq!(dt.take_change(), Some(Some(false)));

        // Multiple changes collapse to the newest.
        tx.send(Some(false)).unwrap();
        tx.send(Some(true)).unwrap();
        assert_eq!(dt.take_change(), Some(Some(true)));

        // Send None (no preference / portal stopped answering).
        tx.send(None).unwrap();
        assert_eq!(dt.take_change(), Some(None));
    }

    /// A new app doesn't watch the desktop (the app binary starts the watcher), so tests and
    /// headless screenshots never depend on the host's theme or spawn D-Bus pollers.
    #[test]
    fn a_new_app_does_not_watch_the_desktop() {
        let app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        assert!(!app.desktop_theme.watching());
    }
}
