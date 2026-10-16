//! The control channel's key file, one rule for the app (which writes it) and the MCP bridge
//! (which reads it): `<settings>/control-key.<instance>`. See `docs/control-protocol.md`, Keys.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A file with this name next to the executable (next to `WordCraft.app` on macOS) makes the
/// install portable: the settings folder is [`PORTABLE_DATA_DIR`] beside it. The Windows portable
/// zip ships one.
const PORTABLE_MARKER: &str = "portable.txt";
const PORTABLE_DATA_DIR: &str = "WordCraftData";

/// The app's settings folder (preferences, logs, key files): macOS
/// `~/Library/Application Support/WordCraft`, Windows `%APPDATA%\WordCraft`, otherwise
/// `$XDG_CONFIG_HOME/wordcraft` or `~/.config/wordcraft`.
/// `WORDCRAFT_PROFILE_DIR` overrides the entire folder for isolated runs. Otherwise a portable
/// install ([`PORTABLE_MARKER`] beside the executable) uses `WordCraftData` there instead. The
/// Windows portable zip has `wordcraft-cli` in the same folder, so the MCP bridge finds the same
/// key files.
pub fn settings_dir() -> Option<PathBuf> {
    settings_dir_with(|key| std::env::var_os(key), executable_dir().as_deref())
}

fn settings_dir_with(env: impl Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = env("WORDCRAFT_PROFILE_DIR").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if let Some(dir) = exe_dir.and_then(portable_dir) {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        env("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/WordCraft"))
    } else if cfg!(windows) {
        env("APPDATA").map(|a| PathBuf::from(a).join("WordCraft"))
    } else {
        env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config"))).map(|c| c.join("wordcraft"))
    }
}

/// `<dir>/WordCraftData` when `<dir>` holds [`PORTABLE_MARKER`], `<dir>` being the folder of the
/// executable or, for a macOS app bundle, the folder of `WordCraft.app`. A folder named like the
/// marker is no marker.
fn portable_dir(exe_dir: &Path) -> Option<PathBuf> {
    let base = bundle_parent(exe_dir).unwrap_or(exe_dir);
    base.join(PORTABLE_MARKER).is_file().then(|| base.join(PORTABLE_DATA_DIR))
}

/// The folder holding `<name>.app` when `exe_dir` is its `Contents/MacOS` (macOS only).
fn bundle_parent(exe_dir: &Path) -> Option<&Path> {
    if !cfg!(target_os = "macos") || exe_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = exe_dir.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    if bundle.extension()? != "app" {
        return None;
    }
    bundle.parent()
}

fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// The `instance-id=` value of a `/.flatpak-info` file, only when it is `[A-Za-z0-9_-]+` (it
/// becomes part of a file name).
pub fn parse_instance_id(info: &str) -> Option<String> {
    let v = info.lines().find_map(|l| l.strip_prefix("instance-id="))?.trim();
    (!v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then(|| v.to_string())
}

/// The Flatpak instance id, or `None` outside Flatpak.
pub fn instance_id() -> Option<String> {
    parse_instance_id(&std::fs::read_to_string("/.flatpak-info").ok()?)
}

/// `<settings>/control-key.<instance>`, with `<instance>` the Flatpak instance id or
/// `local-<port>`.
pub fn key_path(settings: &Path, port: u16) -> PathBuf {
    key_path_for(settings, instance_id().as_deref(), port)
}

fn key_path_for(settings: &Path, instance: Option<&str>, port: u16) -> PathBuf {
    match instance {
        Some(id) => settings.join(format!("control-key.{id}")),
        None => settings.join(format!("control-key.local-{port}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn instance_id_only_safe_values() {
        assert_eq!(parse_instance_id("[Instance]\ninstance-id=abc_123-X\n").as_deref(), Some("abc_123-X"));
        assert_eq!(parse_instance_id("instance-id= 42 \n").as_deref(), Some("42"));
        // It ends up in a file name.
        assert_eq!(parse_instance_id("instance-id=../etc\n"), None);
        assert_eq!(parse_instance_id("instance-id=a/b\n"), None);
        assert_eq!(parse_instance_id("instance-id=a b\n"), None);
        assert_eq!(parse_instance_id("instance-id=..\n"), None);
        assert_eq!(parse_instance_id("instance-id=\n"), None);
        assert_eq!(parse_instance_id("app=ai.storyteller.wordcraft\n"), None);
    }

    #[test]
    fn key_file_per_instance() {
        let dir = Path::new("/settings/wordcraft");
        assert_eq!(key_path_for(dir, Some("1234"), 7981), dir.join("control-key.1234"));
        assert_eq!(key_path_for(dir, None, 7981), dir.join("control-key.local-7981"));
        assert_eq!(key_path_for(dir, None, 7982), dir.join("control-key.local-7982"));
        if instance_id().is_none() {
            assert_eq!(key_path(dir, 7981), dir.join("control-key.local-7981"));
        }
    }

    /// Every platform's convention resolves under `/user`.
    fn environment(key: &str) -> Option<OsString> {
        match key {
            "HOME" => Some("/user".into()),
            "APPDATA" => Some("/user/roaming".into()),
            "XDG_CONFIG_HOME" => Some("/user/xdg".into()),
            _ => None,
        }
    }

    fn platform_dir() -> PathBuf {
        if cfg!(target_os = "macos") {
            "/user/Library/Application Support/WordCraft".into()
        } else if cfg!(windows) {
            Path::new("/user/roaming").join("WordCraft")
        } else {
            "/user/xdg/wordcraft".into()
        }
    }

    /// A folder standing in for the executable's; `marker` puts `portable.txt` in it, or not.
    fn install(name: &str, marker: impl FnOnce(&Path)) -> PathBuf {
        let exe = std::env::temp_dir().join(format!("wordcraft-portable-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&exe);
        std::fs::create_dir_all(&exe).unwrap();
        marker(&exe.join("portable.txt"));
        exe
    }

    #[test]
    fn portable_install_keeps_settings_beside_the_executable() {
        let exe = install("marker", |marker| std::fs::write(marker, "").unwrap());
        let settings = settings_dir_with(environment, Some(&exe)).unwrap();
        assert_eq!(settings, exe.join("WordCraftData"));
        assert_eq!(key_path_for(&settings, None, 7981), exe.join("WordCraftData").join("control-key.local-7981"));
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn a_mac_app_bundle_looks_for_the_marker_beside_the_bundle() {
        let root = install("bundle", |marker| std::fs::write(marker, "").unwrap());
        let exe = root.join("WordCraft.app/Contents/MacOS");
        std::fs::create_dir_all(&exe).unwrap();
        let expected = if cfg!(target_os = "macos") { root.join("WordCraftData") } else { platform_dir() };
        assert_eq!(settings_dir_with(environment, Some(&exe)), Some(expected));
        // On macOS the marker goes beside the bundle; one inside it is not looked for.
        std::fs::remove_file(root.join("portable.txt")).unwrap();
        std::fs::write(exe.join("portable.txt"), "").unwrap();
        let expected = if cfg!(target_os = "macos") { platform_dir() } else { exe.join("WordCraftData") };
        assert_eq!(settings_dir_with(environment, Some(&exe)), Some(expected));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_a_marker_file_the_platform_folder_is_unchanged() {
        let plain = install("plain", |_| {});
        let folder = install("folder", |marker| std::fs::create_dir(marker).unwrap());
        for exe in [plain.as_path(), folder.as_path()] {
            assert_eq!(settings_dir_with(environment, Some(exe)), Some(platform_dir()), "{}", exe.display());
            assert!(!exe.join("WordCraftData").exists());
        }
        assert_eq!(settings_dir_with(environment, None), Some(platform_dir()));
        assert_eq!(settings_dir_with(|_| None, None), None);
        let _ = std::fs::remove_dir_all(&plain);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_profile_override_wins_over_a_portable_install() {
        let exe = install("override", |marker| std::fs::write(marker, "").unwrap());
        let profile = |value: &'static str| move |key: &str| if key == "WORDCRAFT_PROFILE_DIR" { Some(value.into()) } else { environment(key) };
        assert_eq!(settings_dir_with(profile("/isolated/profile"), Some(&exe)), Some(PathBuf::from("/isolated/profile")));
        // An empty override is no override: the marker decides.
        assert_eq!(settings_dir_with(profile(""), Some(&exe)), Some(exe.join("WordCraftData")));
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn settings_dir_is_the_apps_folder() {
        if let Some(d) = settings_dir() {
            let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
            assert!(name == "wordcraft" || name == "WordCraft", "{}", d.display());
        }
    }
}
