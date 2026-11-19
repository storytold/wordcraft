//! The desktop's animation setting on Linux and the BSDs.
//!
//! Asked of the XDG settings portal over D-Bus (`org.freedesktop.portal.Settings`), which GNOME,
//! KDE Plasma and the other desktops with a portal answer, inside Flatpak too. Three settings say
//! whether animations play ([`KEYS`]); any one of them saying no turns them off. Where no portal
//! answers, or it knows none of the three (a bare window manager), the user's own KDE and GTK
//! settings files are read instead, once.
//!
//! A thread asks at start-up and then follows the portal's change signal, so the UI's question is
//! always answered from memory and a change in the desktop's settings shows without a restart.

/// What a setting holds, whichever way it was stored.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// A setting as the portal names it: (namespace, key).
type Key = (&'static str, &'static str);

/// The cross-desktop preference: 1 asks for reduced motion; 0, and values not defined yet, are
/// no preference.
const REDUCED_MOTION: Key = ("org.freedesktop.appearance", "reduced-motion");
/// GNOME's switch, the one GTK follows.
const GNOME_ANIMATIONS: Key = ("org.gnome.desktop.interface", "enable-animations");
/// KDE Plasma's animation speed, a factor on every animation's length: 0 makes them instant.
const KDE_ANIMATION_SPEED: Key = ("org.kde.kdeglobals.KDE", "AnimationDurationFactor");

/// The settings that say whether animations play.
pub const KEYS: [Key; 3] = [REDUCED_MOTION, GNOME_ANIMATIONS, KDE_ANIMATION_SPEED];

/// Whether animations play according to one of [`KEYS`]; `None` for any other setting, or a
/// value that isn't one it holds.
pub fn says(namespace: &str, key: &str, value: &Reading) -> Option<bool> {
    match (namespace, key) {
        k if k == REDUCED_MOTION => number(value).map(|n| n != 1.0),
        k if k == GNOME_ANIMATIONS => truth(value),
        k if k == KDE_ANIMATION_SPEED => number(value).map(|n| n > 0.0),
        _ => None,
    }
}

fn number(value: &Reading) -> Option<f64> {
    let n = match value {
        Reading::Number(n) => *n,
        Reading::Text(t) => t.trim().parse().ok()?,
        Reading::Bool(_) => return None,
    };
    n.is_finite().then_some(n)
}

fn truth(value: &Reading) -> Option<bool> {
    match value {
        Reading::Bool(b) => Some(*b),
        Reading::Number(n) => n.is_finite().then_some(*n != 0.0),
        Reading::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
    }
}

/// Several settings' answers as one: off as soon as one says off, on when at least one says on,
/// `None` when none says anything.
pub fn combine(answers: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut any = None;
    for on in answers.into_iter().flatten() {
        if !on {
            return Some(false);
        }
        any = Some(true);
    }
    any
}

/// The value of `key` in `[section]` of a settings file in INI form (KDE's `kdeglobals`, GTK's
/// `settings.ini`). The last one wins, as the toolkits read it.
pub fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name.trim() == section;
        } else if inside
            && !line.starts_with(['#', ';'])
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            found = Some(v.trim());
        }
    }
    found
}

/// What the user's own settings files say, given their text (`None` for a file that isn't
/// there): KDE Plasma's `kdeglobals` and GTK's `settings.ini`s.
pub fn files_say(kdeglobals: Option<&str>, gtk_settings: [Option<&str>; 2]) -> Option<bool> {
    let (namespace, key) = KDE_ANIMATION_SPEED;
    let kde = kdeglobals.and_then(|text| ini_value(text, "KDE", key)).and_then(|v| says(namespace, key, &Reading::Text(v.to_string())));
    let gtk = gtk_settings
        .into_iter()
        .map(|text| text.and_then(|t| ini_value(t, "Settings", "gtk-enable-animations")).and_then(|v| truth(&Reading::Text(v.to_string()))));
    combine(std::iter::once(kde).chain(gtk))
}

/// The user's desktop settings directory. Relative environment paths must not select files
/// from the working directory instead of the user's KDE and GTK settings.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn config_dir(config_home: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    config_home
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(PathBuf::from).filter(|home| home.is_absolute()).map(|home| home.join(".config")))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub use watching::watch;

#[cfg(all(unix, not(target_os = "macos")))]
mod watching {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    use super::{KEYS, Reading, combine, config_dir, files_say, says};

    // The latest answer, as one byte the UI thread reads without waiting.
    const UNKNOWN: u8 = 0;
    const ON: u8 = 1;
    const OFF: u8 = 2;

    /// Start following the desktop's animation setting. The returned function gives the latest
    /// answer from memory (`None` until the first one arrives, and when the desktop says nothing);
    /// `ctx` is asked to redraw when the answer changes.
    pub fn watch(ctx: egui::Context) -> impl Fn() -> Option<bool> {
        let latest = Arc::new(AtomicU8::new(UNKNOWN));
        let shared = latest.clone();
        let spawned = std::thread::Builder::new().name("animation-setting".into()).spawn(move || {
            let publish = |answer: Option<bool>| {
                let code = match answer {
                    Some(true) => ON,
                    Some(false) => OFF,
                    None => UNKNOWN,
                };
                if shared.swap(code, Ordering::Relaxed) != code {
                    ctx.request_repaint();
                }
            };
            if let Err(e) = follow_portal(&publish) {
                log::debug!("desktop animation setting: no settings portal ({e}); reading the settings files");
                publish(read_files());
            }
        });
        if let Err(e) = spawned {
            log::warn!("desktop animation setting: {e}");
        }
        move || match latest.load(Ordering::Relaxed) {
            ON => Some(true),
            OFF => Some(false),
            _ => None,
        }
    }

    /// Publish what the settings portal says, now and after every change it announces, until the
    /// connection ends; an error when there is no session bus to ask on. While the portal knows
    /// none of the settings, the files' answer stands.
    fn follow_portal(publish: &dyn Fn(Option<bool>)) -> zbus::Result<()> {
        let connection = Connection::session()?;
        let portal = Proxy::new(&connection, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings")?;
        // Listen before the first read, so a change between the two isn't missed.
        let changes = portal.receive_signal("SettingChanged")?;
        // The files are read once, the first time the portal has nothing to say.
        let mut files = None;
        let mut ask = || ask_portal(&portal).or_else(|| *files.get_or_insert_with(read_files));
        publish(ask());
        // A setting can turn up later (KDE's has no value while the speed is at its default), so
        // the changes are followed whatever the portal knew at first. Every setting is asked
        // again, not just the one announced: KDE announces a new animation speed only as the
        // cross-desktop preference, and its own setting must not be left at the old value.
        for change in changes {
            let ours = change
                .body()
                .deserialize::<(String, String, OwnedValue)>()
                .is_ok_and(|(namespace, key, _)| KEYS.contains(&(namespace.as_str(), key.as_str())));
            if ours {
                publish(ask());
            }
        }
        Ok(())
    }

    /// What the portal's settings come to; `None` when it has none of them.
    fn ask_portal(portal: &Proxy<'_>) -> Option<bool> {
        combine(KEYS.map(|(namespace, key)| read_setting(portal, namespace, key).and_then(|value| says(namespace, key, &value))))
    }

    /// One setting from the portal; `None` when it doesn't have it.
    fn read_setting(portal: &Proxy<'_>, namespace: &str, key: &str) -> Option<Reading> {
        // `ReadOne` is the current call; portals from before it only have `Read`.
        let value: OwnedValue = portal.call("ReadOne", &(namespace, key)).or_else(|_| portal.call("Read", &(namespace, key))).ok()?;
        reading(&value, 0)
    }

    fn reading(value: &Value<'_>, depth: u8) -> Option<Reading> {
        Some(match value {
            Value::Bool(b) => Reading::Bool(*b),
            Value::U8(n) => Reading::Number(f64::from(*n)),
            Value::I16(n) => Reading::Number(f64::from(*n)),
            Value::U16(n) => Reading::Number(f64::from(*n)),
            Value::I32(n) => Reading::Number(f64::from(*n)),
            Value::U32(n) => Reading::Number(f64::from(*n)),
            Value::I64(n) => Reading::Number(*n as f64),
            Value::U64(n) => Reading::Number(*n as f64),
            Value::F64(n) => Reading::Number(*n),
            Value::Str(s) => Reading::Text(s.as_str().to_string()),
            // `Read` wraps its answer in a second variant.
            Value::Value(inner) if depth < 4 => return reading(inner, depth + 1),
            _ => return None,
        })
    }

    /// What the user's settings files say (see [`files_say`]).
    fn read_files() -> Option<bool> {
        let config = config_dir(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))?;
        let text = |file: &str| read_capped(&config.join(file));
        let (kdeglobals, gtk4, gtk3) = (text("kdeglobals"), text("gtk-4.0/settings.ini"), text("gtk-3.0/settings.ini"));
        files_say(kdeglobals.as_deref(), [gtk4.as_deref(), gtk3.as_deref()])
    }

    /// A settings file's text: at most 256 KiB of it (they are a few kilobytes).
    fn read_capped(path: &Path) -> Option<String> {
        use std::io::Read as _;
        let mut text = String::new();
        std::fs::File::open(path).ok()?.take(256 * 1024).read_to_string(&mut text).ok()?;
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Reading {
        Reading::Text(s.to_string())
    }

    #[test]
    fn settings_directory_falls_back_from_invalid_xdg_paths() {
        let base = std::env::current_dir().unwrap().join("animation-config-test");
        let xdg = base.join("xdg");
        let home = base.join("home");
        let expected = home.join(".config");
        assert_eq!(config_dir(Some(xdg.clone().into_os_string()), Some(home.clone().into_os_string())), Some(xdg.clone()));
        assert_eq!(config_dir(Some(xdg.clone().into_os_string()), None), Some(xdg));
        for config in [None, Some("".into()), Some("relative-config".into())] {
            assert_eq!(config_dir(config, Some(home.clone().into_os_string())), Some(expected.clone()));
        }
    }

    #[test]
    fn settings_directory_requires_an_absolute_home_fallback() {
        for home in [Some("".into()), None, Some("relative-home".into())] {
            for config in [None, Some("".into()), Some("relative-config".into())] {
                assert_eq!(config_dir(config.clone(), home.clone()), None, "{config:?} {home:?}");
            }
        }
    }

    /// Each desktop's setting, as the portal hands it over, comes to animations on or off.
    #[test]
    fn each_desktop_setting_says_on_or_off() {
        let ((appearance, reduced), (gnome, enable), (kde, factor)) = (REDUCED_MOTION, GNOME_ANIMATIONS, KDE_ANIMATION_SPEED);
        assert_eq!(KEYS, [REDUCED_MOTION, GNOME_ANIMATIONS, KDE_ANIMATION_SPEED]);
        // The cross-desktop preference: only 1 asks for reduced motion.
        assert_eq!(says(appearance, reduced, &Reading::Number(1.0)), Some(false));
        assert_eq!(says(appearance, reduced, &Reading::Number(0.0)), Some(true));
        assert_eq!(says(appearance, reduced, &Reading::Number(7.0)), Some(true), "a value not defined yet is no preference");
        assert_eq!(says(appearance, reduced, &Reading::Bool(true)), None);
        // GNOME's switch.
        assert_eq!(says(gnome, enable, &Reading::Bool(false)), Some(false));
        assert_eq!(says(gnome, enable, &Reading::Bool(true)), Some(true));
        assert_eq!(says(gnome, enable, &text("nonsense")), None);
        // KDE Plasma's animation speed arrives as text: 0 is instant.
        assert_eq!(says(kde, factor, &text("0")), Some(false));
        assert_eq!(says(kde, factor, &text(" 0.5 ")), Some(true));
        assert_eq!(says(kde, factor, &Reading::Number(1.0)), Some(true));
        for junk in [text(""), text("fast"), Reading::Number(f64::NAN), text("inf")] {
            assert_eq!(says(kde, factor, &junk), None, "{junk:?}");
        }
        // Any other setting says nothing.
        assert_eq!(says(gnome, "color-scheme", &Reading::Bool(false)), None);
        assert_eq!(says("org.example", enable, &Reading::Bool(false)), None);
    }

    /// One setting saying off is enough; none saying anything is no answer.
    #[test]
    fn any_setting_saying_off_turns_animations_off() {
        assert_eq!(combine([]), None);
        assert_eq!(combine([None, None]), None);
        assert_eq!(combine([None, Some(true)]), Some(true));
        assert_eq!(combine([Some(true), None, Some(false)]), Some(false));
        assert_eq!(combine([Some(false), Some(true)]), Some(false));
    }

    /// Without a portal the user's own settings files answer: KDE's animation speed and GTK's
    /// `gtk-enable-animations`.
    #[test]
    fn settings_files_answer_without_a_portal() {
        let kde_instant = "[General]\nColorScheme=Breeze\n\n[KDE]\nAnimationDurationFactor=0\nSingleClick=false\n";
        let kde_slow = "[KDE]\nAnimationDurationFactor = 2\n";
        let gtk_off = "[Settings]\ngtk-theme-name=Adwaita\ngtk-enable-animations=0\n";
        let gtk_on = "# written by hand\n[Settings]\ngtk-enable-animations=true\n";
        assert_eq!(files_say(None, [None, None]), None);
        assert_eq!(files_say(Some("[KDE]\nSingleClick=false\n"), [Some("[Settings]\n"), None]), None, "files that don't mention it");
        assert_eq!(files_say(Some(kde_instant), [None, None]), Some(false));
        assert_eq!(files_say(Some(kde_slow), [None, None]), Some(true));
        assert_eq!(files_say(None, [Some(gtk_off), None]), Some(false));
        assert_eq!(files_say(None, [None, Some(gtk_on)]), Some(true));
        assert_eq!(files_say(Some(kde_slow), [Some(gtk_on), Some(gtk_off)]), Some(false), "one file saying off is enough");

        // The key counts only in its own section, comments don't, and the last value wins.
        assert_eq!(ini_value("[Other]\nAnimationDurationFactor=0\n[KDE]\nSingleClick=true\n", "KDE", "AnimationDurationFactor"), None);
        assert_eq!(ini_value("[KDE]\n#AnimationDurationFactor=0\n;AnimationDurationFactor=0\n", "KDE", "AnimationDurationFactor"), None);
        assert_eq!(ini_value("[KDE]\r\nAnimationDurationFactor=0\r\nAnimationDurationFactor=1\r\n", "KDE", "AnimationDurationFactor"), Some("1"));
        assert_eq!(ini_value("AnimationDurationFactor=0\n", "KDE", "AnimationDurationFactor"), None, "outside any section");
        assert_eq!(ini_value("", "KDE", "AnimationDurationFactor"), None);
    }
}
