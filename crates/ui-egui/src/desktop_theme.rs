//! The desktop's own light/dark choice, for an interface setting that follows it.
//!
//! winit reports no system theme on Linux — `ActiveEventLoop::system_theme` is `None` on every
//! desktop there — so "Use system setting" has to ask the desktop itself. The XDG Desktop Portal's
//! Settings interface answers on GNOME, KDE and XFCE alike, over X11 and Wayland alike.

use std::sync::mpsc::{Receiver, TryRecvError};
#[cfg(target_os = "linux")]
use std::sync::{
    Arc, Mutex,
    mpsc::{Sender, channel},
};
#[cfg(target_os = "linux")]
use std::time::Duration;

/// Retry only after a connection, subscription or read failure; a healthy watcher never polls.
#[cfg(target_os = "linux")]
const RETRY: Duration = Duration::from_secs(4);

#[cfg(target_os = "linux")]
const PORTAL: &str = "org.freedesktop.portal.Desktop";
#[cfg(target_os = "linux")]
const SETTINGS: &str = "org.freedesktop.portal.Settings";
#[cfg(target_os = "linux")]
const SCHEMAS: [&str; 3] = ["org.freedesktop.appearance", "org.gnome.desktop.interface", "org.freedesktop.interface.color-scheme"];

/// Whether the desktop asks applications to draw dark; no preference resolves to light.
/// `None` when the setting is unrecognized or unavailable (no session bus or portal), so the
/// caller keeps whatever it chose itself.
pub fn prefers_dark() -> Option<bool> {
    read(&mut Bus::new())
}

/// Watches the desktop and reports its choice when it changes. [`prefers_dark`] covers the
/// answer at start-up; this covers the user switching theme while the app runs.
#[derive(Default)]
pub struct DesktopTheme {
    /// `None` when no watcher thread could be started; the one-shot read still works.
    changes: Option<Receiver<Option<bool>>>,
    #[cfg(target_os = "linux")]
    stop: Option<Sender<()>>,
    #[cfg(target_os = "linux")]
    state: Arc<Mutex<WatchState>>,
}

impl DesktopTheme {
    /// Start watching. Nothing runs before this is called, and calling it once is enough. The
    /// first available answer is sent straight away, so the caller does not have to ask separately.
    /// `changed` runs on the watcher's thread after each answer is sent (the app asks for a
    /// repaint there, so an idle window picks the change up). The app binary starts this; tests
    /// and headless screenshots don't, so they never depend on the machine's desktop theme.
    pub fn start(changed: impl Fn() + Send + 'static) -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::start_with(changed, zbus::blocking::Connection::session, RETRY)
        }
        #[cfg(not(target_os = "linux"))]
        {
            // winit supplies the theme on these platforms; no desktop-bus worker is needed.
            let _ = changed;
            Self::default()
        }
    }

    /// Inject the connection factory and retry delay for isolated watcher tests.
    #[cfg(target_os = "linux")]
    fn start_with(
        changed: impl Fn() + Send + 'static,
        connect: impl Fn() -> zbus::Result<zbus::blocking::Connection> + Send + 'static,
        retry: Duration,
    ) -> Self {
        let (tx, rx) = channel();
        let (stop, stopped) = channel();
        let state = Arc::new(Mutex::new(WatchState::default()));
        let worker_state = state.clone();
        let started = std::thread::Builder::new().name("wordcraft-desktop-theme".into()).spawn(move || {
            let mut last = None;
            while watch(&connect, &worker_state, &mut last, &tx, &changed).is_err() {
                // Disconnection wakes this wait too, so dropping the watcher never waits for RETRY.
                if stopped.recv_timeout(retry) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                    return;
                }
            }
        });
        Self { changes: started.ok().map(|_| rx), stop: Some(stop), state }
    }

    /// Whether a watcher thread is running.
    pub fn watching(&self) -> bool {
        self.changes.is_some()
    }

    /// The desktop's new answer if it changed since the last call, to be polled once a frame.
    /// Several changes in one frame collapse to the newest, and a desktop that stops answering
    /// leaves the last answer in place.
    pub fn take_change(&mut self) -> Option<bool> {
        let Some(changes) = &self.changes else { return None };
        let mut newest = None;
        loop {
            match changes.try_recv() {
                Ok(changed) => newest = changed,
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return newest,
            }
        }
    }
}

/// Closing the dedicated connection wakes a worker blocked on an idle message iterator.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct WatchState {
    stopped: bool,
    connection: Option<zbus::blocking::Connection>,
}

#[cfg(target_os = "linux")]
impl Drop for DesktopTheme {
    fn drop(&mut self) {
        let connection = self.state.lock().ok().and_then(|mut state| {
            state.stopped = true;
            state.connection.take()
        });
        self.stop.take();
        if let Some(connection) = connection {
            let _ = connection.close();
        }
    }
}

/// Subscribe before the initial read so a concurrent switch cannot be missed. The dedicated
/// connection receives only the two registered signal kinds (plus replies to its own reads).
/// NameOwnerChanged makes a restarted portal visible even if it emits no initial SettingChanged.
#[cfg(target_os = "linux")]
fn watch(
    connect: &impl Fn() -> zbus::Result<zbus::blocking::Connection>,
    state: &Mutex<WatchState>,
    last: &mut Option<bool>,
    tx: &Sender<Option<bool>>,
    changed: &impl Fn(),
) -> zbus::Result<()> {
    use zbus::{MatchRule, blocking::MessageIterator, message::Type};
    let connection = connect()?;
    {
        let mut state = state.lock().map_err(|_| zbus::Error::Failure("desktop theme watcher poisoned".into()))?;
        if state.stopped {
            return Ok(());
        }
        state.connection = Some(connection.clone());
    }
    let messages = MessageIterator::from(&connection);
    let dbus = zbus::blocking::fdo::DBusProxy::new(&connection)?;
    for (sender, path, interface, member, arg_index, arg_value) in [
        (PORTAL, "/org/freedesktop/portal/desktop", SETTINGS, "SettingChanged", 1, "color-scheme"),
        ("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameOwnerChanged", 0, PORTAL),
    ] {
        dbus.add_match_rule(
            MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(sender)?
                .path(path)?
                .interface(interface)?
                .member(member)?
                .arg(arg_index, arg_value)?
                .build(),
        )?;
    }
    let mut report = || {
        let mut bus = Bus(Some(connection.clone()));
        let answer = read(&mut bus);
        if bus.0.is_none() {
            return Err(zbus::Error::Failure("desktop theme read failed".into()));
        }
        Ok(publish(answer, last, tx, changed))
    };
    if !report()? {
        return Ok(());
    }
    for message in messages {
        if theme_event(&message?) && !report()? {
            return Ok(());
        }
    }
    Err(zbus::Error::Failure("desktop theme connection closed".into()))
}

/// Reread the authoritative setting on relevant signals, preserving standard-key precedence.
/// Ignore malformed signals and unrelated settings without any desktop read or repaint.
#[cfg(target_os = "linux")]
fn theme_event(message: &zbus::Message) -> bool {
    let header = message.header();
    if header.message_type() != zbus::message::Type::Signal {
        return false;
    }
    match (header.interface().map(|v| v.as_str()), header.member().map(|v| v.as_str())) {
        (Some(SETTINGS), Some("SettingChanged")) => message
            .body()
            .deserialize::<(&str, &str, zbus::zvariant::Value<'_>)>()
            .is_ok_and(|(schema, key, _)| SCHEMAS.contains(&schema) && key == "color-scheme"),
        (Some("org.freedesktop.DBus"), Some("NameOwnerChanged")) => {
            message.body().deserialize::<(&str, &str, &str)>().is_ok_and(|(name, _, owner)| name == PORTAL && !owner.is_empty())
        }
        _ => false,
    }
}

/// Keep the last known choice when the desktop is unavailable, and wake only on a real change.
#[cfg(target_os = "linux")]
fn publish(answer: Option<bool>, last: &mut Option<bool>, tx: &Sender<Option<bool>>, changed: &impl Fn()) -> bool {
    let Some(answer) = answer else { return true };
    if *last == Some(answer) {
        return true;
    }
    *last = Some(answer);
    if tx.send(Some(answer)).is_err() {
        return false;
    }
    changed();
    true
}

/// A session-bus connection for reading the current setting.
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

/// Ask the portal over `bus`'s connection; blocking calls belong off the UI thread.
/// Missing settings/services keep the subscription; transient failures discard it for a retry.
#[cfg(target_os = "linux")]
fn read(bus: &mut Bus) -> Option<bool> {
    if bus.0.is_none() {
        bus.0 = zbus::blocking::Connection::session().ok();
    }
    let connection = bus.0.as_ref()?;
    let mut failed = false;
    for schema in SCHEMAS {
        let reply = match connection
            .call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(SETTINGS), "ReadOne", &(schema, "color-scheme"))
            .or_else(|error| match error {
                zbus::Error::MethodError(name, _, _) if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" => {
                    connection.call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(SETTINGS), "Read", &(schema, "color-scheme"))
                }
                error => Err(error),
            }) {
            Ok(reply) => reply,
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(name.as_str(), "org.freedesktop.portal.Error.NotFound" | "org.freedesktop.DBus.Error.ServiceUnknown") =>
            {
                continue;
            }
            Err(_) => {
                failed = true;
                continue;
            }
        };
        let Ok((answer,)) = reply.body().deserialize::<(zbus::zvariant::OwnedValue,)>() else { continue };
        // Settings v1's deprecated Read wraps its answer in an additional variant.
        let answer = match &*answer {
            zbus::zvariant::Value::Value(inner) => inner.as_ref(),
            answer => answer,
        };
        if let Some(dark) = parse(answer) {
            return Some(dark);
        }
    }
    if failed {
        bus.0 = None;
    }
    None
}

/// The standard portal answer is a u32 (0 no preference, 1 dark, 2 light); unknown numbers
/// also mean no preference. Resolve no preference to light, like egui's System fallback,
/// so returning to the default after dark does not leave the last dark answer in place.
/// Older GNOME/XFCE portals expose strings instead.
#[cfg(target_os = "linux")]
fn parse(answer: &zbus::zvariant::Value<'_>) -> Option<bool> {
    if let Ok(scheme) = u32::try_from(answer) {
        return Some(scheme == 1);
    }
    match <&str>::try_from(answer).ok()? {
        "prefer-dark" => Some(true),
        "prefer-light" | "default" => Some(false),
        _ => None,
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

    #[test]
    #[cfg(target_os = "linux")]
    fn notifications_preserve_unknown_answers_and_do_not_repaint_duplicates() {
        let (tx, rx) = channel();
        let mut last = None;
        let repaints = std::cell::Cell::new(0);
        let changed = || repaints.set(repaints.get() + 1);
        assert!(publish(Some(false), &mut last, &tx, &changed));
        assert_eq!(rx.try_recv().unwrap(), Some(false));
        assert!(publish(Some(false), &mut last, &tx, &changed));
        assert!(publish(None, &mut last, &tx, &changed));
        assert_eq!(last, Some(false));
        assert!(rx.try_recv().is_err());
        assert!(publish(Some(true), &mut last, &tx, &changed));
        assert_eq!(rx.try_recv().unwrap(), Some(true));
        assert_eq!(repaints.get(), 2);
        drop(rx);
        assert!(!publish(Some(false), &mut last, &tx, &changed));
        assert_eq!(repaints.get(), 2);
    }

    /// Standard portal values and the legacy GNOME values share the same light fallback.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_portal_answer_reads_as_a_choice_or_as_none() {
        use zbus::zvariant::Value;
        for (scheme, dark) in [(0_u32, false), (1, true), (2, false), (u32::MAX, false)] {
            assert_eq!(parse(&Value::from(scheme)), Some(dark));
        }
        assert_eq!(parse(&Value::from("prefer-dark")), Some(true));
        assert_eq!(parse(&Value::from("prefer-light")), Some(false));
        assert_eq!(parse(&Value::from("default")), Some(false));
        assert_eq!(parse(&Value::from("")), None);
        assert_eq!(parse(&Value::from(true)), None);
    }

    /// Changes travel through the watcher to the full interface, including dark -> default.
    /// A manual appearance stays fixed while the desktop changes.
    #[test]
    #[cfg(target_os = "linux")]
    fn desktop_changes_update_the_interface_only_when_following_system() {
        use crate::theme::{Appearance, Tokens};
        use zbus::zvariant::Value;
        let ctx = egui::Context::default();
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        let (tx, rx) = channel();
        app.desktop_theme.changes = Some(rx);
        for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
            app.run("ui.theme", serde_json::json!({"value": appearance.code()})).unwrap();
            for answer in
                [Value::from(1_u32), Value::from(2_u32), Value::from(1_u32), Value::from(0_u32), Value::from("prefer-dark"), Value::from("default")]
            {
                let dark = parse(&answer).unwrap();
                tx.send(Some(dark)).unwrap();
                ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
                let expected = appearance == Appearance::Dark || (appearance == Appearance::System && dark);
                assert_eq!(Tokens::get(&ctx).dark, expected, "{appearance:?}, {answer:?}");
                assert_eq!(app.ui_is_dark(), expected);
                let palette = if expected { Tokens::dark() } else { Tokens::light() };
                assert_eq!(ctx.global_style().visuals.panel_fill, palette.ribbon);
            }
        }
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

    /// A new app doesn't watch the desktop (the app binary starts the watcher), so tests and
    /// headless screenshots never depend on the host's theme or spawn D-Bus pollers.
    #[test]
    fn a_new_app_does_not_watch_the_desktop() {
        let app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        assert!(!app.desktop_theme.watching());
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
#[path = "desktop_theme_tests.rs"]
mod integration_tests;
