//! Portal appearance, without UI dependencies or synchronous UI-thread D-Bus calls.
//! A bounded two-second poll also recovers from portal restarts. Dropping the handle
//! wakes the worker immediately; no permanently blocked signal listener is leaked.

/// Last successfully decoded desktop preferences, with independent optional fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Appearance {
    /// Explicit dark/light preference; `None` means the desktop expresses no preference.
    pub dark: Option<bool>,
    /// sRGB channels in 0..=1. Absent/invalid values leave the application's accent intact.
    pub accent: Option<[f64; 3]>,
}

/// Nonblocking UI-side receiver; dropping it asks the background poller to stop.
#[derive(Default)]
pub struct Watcher {
    changes: Option<std::sync::mpsc::Receiver<Appearance>>,
    _stop: Option<std::sync::mpsc::Sender<()>>,
}

impl Watcher {
    /// Start bounded portal reads off the UI thread and request repaint on changes.
    /// Unsupported platforms/features or thread creation failure return an inert watcher.
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        #[cfg(all(target_os = "linux", feature = "portal"))]
        {
            use std::sync::mpsc;
            let (tx, changes) = mpsc::channel();
            let (stop, cancelled) = mpsc::channel();
            let started = std::thread::Builder::new()
                .name("craft-appearance".into())
                .spawn(move || {
                    let mut bus = None;
                    let mut previous = None;
                    loop {
                        if let Some(current) = read(&mut bus, previous.unwrap_or_default())
                            && previous != Some(current)
                        {
                            previous = Some(current);
                            if tx.send(current).is_err() {
                                return;
                            }
                            repaint();
                        }
                        match cancelled.recv_timeout(std::time::Duration::from_secs(2)) {
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            _ => return,
                        }
                    }
                });
            if started.is_ok() {
                Self {
                    changes: Some(changes),
                    _stop: Some(stop),
                }
            } else {
                Self::default()
            }
        }
        #[cfg(not(all(target_os = "linux", feature = "portal")))]
        {
            let _ = repaint;
            Self::default()
        }
    }

    /// Whether a polling worker was started, not a guarantee that the portal is available.
    pub fn watching(&self) -> bool {
        self.changes.is_some()
    }

    /// Drain pending notifications and return only the newest snapshot without waiting.
    pub fn take_change(&mut self) -> Option<Appearance> {
        self.changes.as_ref()?.try_iter().last()
    }
}

/// Interpret standard portal codes: dark (1), light (2), otherwise no preference.
pub fn decode_scheme(value: u32) -> Option<bool> {
    match value {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Accept only finite sRGB channels in the inclusive unit interval.
pub fn validate_accent(rgb: [f64; 3]) -> Option<[f64; 3]> {
    rgb.iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .then_some(rgb)
}

#[cfg(all(target_os = "linux", feature = "portal"))]
/// Remove at most four variant wrappers used by older Settings portal implementations.
fn unpack(mut value: zbus::zvariant::Value<'_>) -> zbus::zvariant::Value<'_> {
    for _ in 0..4 {
        match value {
            zbus::zvariant::Value::Value(inner) => value = *inner,
            other => return other,
        }
    }
    value
}

#[cfg(all(target_os = "linux", feature = "portal"))]
/// Read both fields independently; retry the connection after total portal failure.
fn read(bus: &mut Option<zbus::blocking::Connection>, previous: Appearance) -> Option<Appearance> {
    use zbus::{blocking::connection::Builder, zvariant::OwnedValue};
    if bus.is_none() {
        *bus = Builder::session()
            .ok()?
            .method_timeout(std::time::Duration::from_secs(1))
            .build()
            .ok();
    }
    let connection = bus.as_ref()?;
    let query = |key| -> Option<OwnedValue> {
        // Read is supported by older Settings implementations too. It may wrap a variant twice.
        connection
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "Read",
                &("org.freedesktop.appearance", key),
            )
            .ok()?
            .body()
            .deserialize()
            .ok()
    };
    let scheme = query("color-scheme");
    let accent = query("accent-color");
    if scheme.is_none() && accent.is_none() {
        *bus = None;
        return None; // Keep the last successful answer while the portal is restarting.
    }
    Some(decode_appearance(scheme, accent, previous))
}

/// Preserve each field on a failed/malformed read, but honor a successfully decoded
/// reset (scheme 0 or an invalid RGB range). `None` after decoding is not a read error.
#[cfg(all(target_os = "linux", feature = "portal"))]
fn decode_appearance(
    scheme: Option<zbus::zvariant::OwnedValue>,
    accent: Option<zbus::zvariant::OwnedValue>,
    previous: Appearance,
) -> Appearance {
    Appearance {
        dark: scheme
            .and_then(|v| u32::try_from(unpack(v.into())).ok())
            .map(decode_scheme)
            .unwrap_or(previous.dark),
        accent: accent
            .and_then(|v| <(f64, f64, f64)>::try_from(unpack(v.into())).ok())
            .map(|(r, g, b)| validate_accent([r, g, b]))
            .unwrap_or(previous.accent),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[cfg(all(target_os = "linux", feature = "portal"))]
    struct SettingsState {
        scheme: Option<u32>,
        accent: Option<(f64, f64, f64)>,
    }
    #[cfg(all(target_os = "linux", feature = "portal"))]
    struct Settings(std::sync::Arc<std::sync::Mutex<SettingsState>>);

    #[cfg(all(target_os = "linux", feature = "portal"))]
    #[zbus::interface(name = "org.freedesktop.portal.Settings")]
    impl Settings {
        fn read(
            &self,
            namespace: &str,
            key: &str,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedValue> {
            if namespace != "org.freedesktop.appearance" {
                return Err(zbus::fdo::Error::InvalidArgs("namespace".into()));
            }
            let state = self
                .0
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            let missing = || zbus::fdo::Error::Failed("synthetic partial read failure".into());
            match key {
                "color-scheme" => state.scheme.map(Into::into).ok_or_else(missing),
                "accent-color" => zbus::zvariant::OwnedValue::try_from(
                    zbus::zvariant::Value::from(state.accent.ok_or_else(missing)?),
                )
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string())),
                _ => Err(zbus::fdo::Error::InvalidArgs("key".into())),
            }
        }
    }

    #[test]
    #[ignore = "requires a private D-Bus session; never run against the user's portal"]
    #[cfg(all(target_os = "linux", feature = "portal"))]
    #[allow(clippy::unwrap_used)]
    fn appearance_dbus_live_changes_and_drop_stop_worker() {
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
        let value = Arc::new(Mutex::new(SettingsState {
            scheme: Some(1),
            accent: Some((0.2, 0.4, 0.6)),
        }));
        let _server = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Settings(value.clone()))
            .unwrap()
            .build()
            .unwrap();
        let marker = Arc::new(());
        let weak = Arc::downgrade(&marker);
        let mut watcher = Watcher::start(move || {
            let _ = &marker;
        });
        let rx = watcher.changes.as_mut().unwrap();
        let first = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(first.dark, Some(true));
        assert_eq!(first.accent, Some([0.2, 0.4, 0.6]));
        value.lock().unwrap().scheme = Some(2);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(4)).unwrap().dark,
            Some(false)
        );
        *value.lock().unwrap() = SettingsState {
            scheme: None,
            accent: Some((0.3, 0.5, 0.7)),
        };
        let partial = rx.recv_timeout(Duration::from_secs(4)).unwrap();
        assert_eq!(
            partial.dark,
            Some(false),
            "failed scheme read must keep light mode"
        );
        assert_eq!(partial.accent, Some([0.3, 0.5, 0.7]));
        *value.lock().unwrap() = SettingsState {
            scheme: Some(0),
            accent: None,
        };
        let reset = rx.recv_timeout(Duration::from_secs(4)).unwrap();
        assert_eq!(reset.dark, None, "successful zero resets the preference");
        assert_eq!(
            reset.accent, partial.accent,
            "failed accent read preserves the last value"
        );
        drop(watcher);
        let deadline = Instant::now() + Duration::from_secs(3);
        while weak.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            weak.upgrade().is_none(),
            "watcher must stop even if appearance never changes again"
        );
    }
    #[test]
    fn standard_portal_codes_and_finite_accent() {
        assert_eq!(decode_scheme(0), None);
        assert_eq!(decode_scheme(1), Some(true));
        assert_eq!(decode_scheme(2), Some(false));
        assert_eq!(decode_scheme(99), None);
        assert_eq!(validate_accent([0., 0.5, 1.]), Some([0., 0.5, 1.]));
        assert_eq!(validate_accent([f64::NAN, 0., 0.]), None);
        assert_eq!(validate_accent([0., 1.1, 0.]), None);
    }
    #[test]
    fn default_does_not_access_desktop() {
        let mut watcher = Watcher::default();
        assert!(!watcher.watching());
        assert_eq!(watcher.take_change(), None);
    }
    #[test]
    #[cfg(all(target_os = "linux", feature = "portal"))]
    fn nested_variants_are_supported() {
        use zbus::zvariant::Value;
        assert_eq!(
            u32::try_from(unpack(Value::Value(Box::new(Value::U32(1))))).ok(),
            Some(1)
        );
    }

    /// Missing or mistyped portal fields preserve their own last answer, not the whole snapshot.
    #[test]
    #[cfg(all(target_os = "linux", feature = "portal"))]
    fn partial_appearance_reads_preserve_only_failed_fields() {
        use zbus::zvariant::{OwnedValue, Value};
        let previous = Appearance {
            dark: Some(true),
            accent: Some([0.2, 0.4, 0.6]),
        };
        let accent = OwnedValue::try_from(Value::from((0.3, 0.5, 0.7))).unwrap();
        assert_eq!(
            decode_appearance(None, Some(accent), previous),
            Appearance {
                dark: Some(true),
                accent: Some([0.3, 0.5, 0.7])
            }
        );
        assert_eq!(
            decode_appearance(Some(2u32.into()), None, previous),
            Appearance {
                dark: Some(false),
                ..previous
            }
        );
        assert_eq!(decode_appearance(None, None, previous), previous);
        assert_eq!(
            decode_appearance(Some(true.into()), Some(true.into()), previous),
            previous
        );
    }

    /// An explicit no-preference answer must not be mistaken for a failed read.
    #[test]
    #[cfg(all(target_os = "linux", feature = "portal"))]
    fn successful_appearance_resets_are_not_replaced_with_stale_values() {
        use zbus::zvariant::{OwnedValue, Value};
        let previous = Appearance {
            dark: Some(true),
            accent: Some([0.2, 0.4, 0.6]),
        };
        let invalid = OwnedValue::try_from(Value::from((1.2, 0.5, 0.7))).unwrap();
        assert_eq!(
            decode_appearance(Some(0u32.into()), Some(invalid), previous),
            Appearance::default()
        );
        assert_eq!(
            decode_appearance(Some(99u32.into()), None, previous).dark,
            None
        );
    }
}
