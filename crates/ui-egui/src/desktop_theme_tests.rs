//! Exercise the real subscriptions on a private bus, never the user's desktop session.
use super::*;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use zbus::blocking::Connection;

struct PrivateBus(Child, String);

impl PrivateBus {
    /// Start an isolated broker, optionally reusing its address after a restart.
    fn start(address: Option<&str>) -> Self {
        let mut command = Command::new("dbus-daemon");
        command.args(["--session", "--nofork", "--print-address=1"]).stdout(Stdio::piped());
        if let Some(address) = address {
            command.arg(format!("--address={address}"));
        }
        let mut child = command.spawn().expect("Linux theme integration tests require dbus-daemon");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut address).unwrap();
        Self(child, address.trim().split(",guid=").next().unwrap().into())
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        if let Some(path) = self.1.strip_prefix("unix:path=") {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Default)]
struct Setting {
    dark: bool,
    error: Option<PortalError>,
    switch_during_read: bool,
    reads: usize,
}

#[derive(Clone, Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop")]
enum PortalError {
    #[zbus(name = "portal.Error.NotFound")]
    NotFound(String),
    #[zbus(name = "DBus.Error.ServiceUnknown")]
    ServiceUnknown(String),
    #[zbus(name = "DBus.Error.Failed")]
    Failed(String),
    #[zbus(error)]
    ZBus(zbus::Error),
}

struct Portal(Arc<Mutex<Setting>>);

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Portal {
    async fn read_one(
        &self,
        namespace: &str,
        key: &str,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> Result<zbus::zvariant::OwnedValue, PortalError> {
        let (dark, switch) = {
            let mut setting = self.0.lock().unwrap();
            setting.reads += 1;
            if namespace != SCHEMAS[0] || key != "color-scheme" {
                return Err(PortalError::NotFound(key.into()));
            }
            if let Some(error) = &setting.error {
                return Err(error.clone());
            }
            let answer = (setting.dark, setting.switch_during_read);
            if setting.switch_during_read {
                setting.dark = false;
                setting.switch_during_read = false;
            }
            answer
        };
        if switch {
            emitter.emit(SETTINGS, "SettingChanged", &(SCHEMAS[0], "color-scheme", zbus::zvariant::Value::from(2_u32))).await?;
        }
        Ok(zbus::zvariant::OwnedValue::from(if dark { 1_u32 } else { 2_u32 }))
    }
}

/// Export a controllable Settings service before acquiring the portal name.
fn portal(bus: &PrivateBus, setting: &Arc<Mutex<Setting>>) -> Connection {
    zbus::blocking::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", Portal(setting.clone()))
        .unwrap()
        .name(PORTAL)
        .unwrap()
        .build()
        .unwrap()
}

/// Notify of a setting change; the watcher must reread rather than trust the signal value.
fn emit(connection: &Connection, namespace: &str, key: &str) {
    connection
        .emit_signal(
            None::<&str>,
            "/org/freedesktop/portal/desktop",
            SETTINGS,
            "SettingChanged",
            &(namespace, key, zbus::zvariant::Value::from(1_u32)),
        )
        .unwrap();
}

/// Bound asynchronous checks so a broken watcher fails rather than hanging.
fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "theme watcher timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn subscriptions_recover_reads_and_restarts_without_idle_polling() {
    let mut bus = PrivateBus::start(None);
    let setting = Arc::new(Mutex::new(Setting { dark: true, switch_during_read: true, ..Default::default() }));
    let mut service = portal(&bus, &setting);
    let address = bus.1.clone();
    let watcher =
        DesktopTheme::start_with(|| {}, move || zbus::blocking::connection::Builder::address(address.as_str())?.build(), Duration::from_millis(20));
    let changes = watcher.changes.as_ref().unwrap();
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(true));
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(false));

    // Legacy notifications still trigger an authoritative read of the standard setting.
    for namespace in &SCHEMAS[1..] {
        let count = setting.lock().unwrap().reads;
        emit(&service, namespace, "color-scheme");
        wait_until(|| setting.lock().unwrap().reads > count);
    }
    let count = setting.lock().unwrap().reads;
    service.emit_signal(None::<&str>, "/org/freedesktop/portal/desktop", SETTINGS, "SettingChanged", &"bad body").unwrap();
    emit(&service, SCHEMAS[0], "contrast");
    emit(&service, "unrelated", "color-scheme");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(setting.lock().unwrap().reads, count, "idle and unrelated signals must not read");

    setting.lock().unwrap().dark = true;
    setting.lock().unwrap().error = Some(PortalError::Failed("temporarily unavailable".into()));
    emit(&service, SCHEMAS[0], "color-scheme");
    wait_until(|| setting.lock().unwrap().reads >= count + 3);
    setting.lock().unwrap().error = None;
    // No further signal: recovery must come from retrying the failed read.
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(true));

    service.close().unwrap();
    setting.lock().unwrap().dark = false;
    service = portal(&bus, &setting);
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(false));

    let address = bus.1.clone();
    drop(bus);
    setting.lock().unwrap().dark = true;
    bus = PrivateBus::start(Some(&address));
    service.close().ok();
    service = portal(&bus, &setting);
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(true));
    setting.lock().unwrap().dark = false;
    emit(&service, SCHEMAS[0], "color-scheme");
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(false));
}

#[test]
fn dropping_an_idle_or_retrying_watcher_releases_its_callback() {
    let bus = PrivateBus::start(None);
    let service = portal(&bus, &Arc::new(Mutex::new(Setting::default())));
    for fail in [false, true] {
        let released = Arc::new(());
        let callback = released.clone();
        let address = bus.1.clone();
        let (attempted, attempts) = channel();
        let watcher = DesktopTheme::start_with(
            move || {
                let _ = &callback;
            },
            move || {
                let _ = attempted.send(());
                if fail {
                    return Err(zbus::Error::Failure("test connection failure".into()));
                }
                zbus::blocking::connection::Builder::address(address.as_str())?.build()
            },
            RETRY,
        );
        attempts.recv_timeout(Duration::from_secs(3)).unwrap();
        if !fail {
            watcher.changes.as_ref().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
        }
        drop(watcher);
        wait_until(|| Arc::strong_count(&released) == 1);
    }
    service.close().unwrap();
}

/// Missing settings/services keep the subscription; transient failures still request a retry.
#[test]
fn permanent_errors_keep_the_bus_but_transient_failures_clear_it() {
    let bus = PrivateBus::start(None);
    let setting = Arc::new(Mutex::new(Setting::default()));
    let _service = portal(&bus, &setting);
    let mut reader = Bus(Some(zbus::blocking::connection::Builder::address(bus.1.as_str()).unwrap().build().unwrap()));
    for error in [PortalError::NotFound("missing setting".into()), PortalError::ServiceUnknown("missing service".into())] {
        setting.lock().unwrap().error = Some(error);
        assert_eq!(read(&mut reader), None);
        assert!(reader.0.is_some(), "permanent errors must preserve the connection");
    }
    // The standard read fails transiently while the legacy keys return NotFound.
    setting.lock().unwrap().error = Some(PortalError::Failed("temporarily unavailable".into()));
    assert_eq!(read(&mut reader), None);
    assert!(reader.0.is_none(), "a missing legacy key must not hide a transient failure");
}

/// Settings v1 has only Read, whose reply contains an extra variant layer.
struct LegacyPortal(Arc<Mutex<Setting>>);

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl LegacyPortal {
    fn read(&self, namespace: &str, key: &str) -> Result<zbus::zvariant::OwnedValue, PortalError> {
        let mut setting = self.0.lock().unwrap();
        setting.reads += 1;
        if namespace != SCHEMAS[0] || key != "color-scheme" {
            return Err(PortalError::NotFound(key.into()));
        }
        Ok(zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::from(if setting.dark { 1_u32 } else { 2_u32 }))).try_into().unwrap())
    }
}

/// A v1 portal supplies both the initial theme and live updates without reconnecting.
#[test]
fn read_only_portal_updates_the_theme_without_reconnecting() {
    let bus = PrivateBus::start(None);
    let setting = Arc::new(Mutex::new(Setting { dark: true, ..Default::default() }));
    let service = zbus::blocking::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", LegacyPortal(setting.clone()))
        .unwrap()
        .name(PORTAL)
        .unwrap()
        .build()
        .unwrap();
    let address = bus.1.clone();
    let (connected, connections) = channel();
    let watcher = DesktopTheme::start_with(
        || {},
        move || {
            connected.send(()).unwrap();
            zbus::blocking::connection::Builder::address(address.as_str())?.build()
        },
        Duration::from_millis(20),
    );
    let changes = watcher.changes.as_ref().unwrap();
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(true));
    connections.recv_timeout(Duration::from_secs(3)).unwrap();
    setting.lock().unwrap().dark = false;
    emit(&service, SCHEMAS[0], "color-scheme");
    assert_eq!(changes.recv_timeout(Duration::from_secs(3)).unwrap(), Some(false));
    assert_eq!(connections.recv_timeout(Duration::from_millis(100)), Err(std::sync::mpsc::RecvTimeoutError::Timeout));
    assert_eq!(setting.lock().unwrap().reads, 2, "idle watchers must not reread settings");
}
