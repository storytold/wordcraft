//! The control channel's token file, one rule for the app (which writes it) and the MCP bridge
//! (which reads it): `<settings>/control-key.<instance>`. See `docs/control-protocol.md`, Token.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};

/// The folder the desktop app chose before any of its state was read: a Launcher-selected
/// Nightly profile (`launch_profile`). Set once, at startup, before the first [`settings_dir`]
/// call; tests and the CLI leave it unset.
static CHOSEN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Point [`settings_dir`] at `dir` for this process. Fails when a folder was already chosen.
pub fn use_settings_dir(dir: PathBuf) -> Result<(), String> {
    CHOSEN.set(dir).map_err(|_| "the settings folder was already selected".into())
}

/// The app's settings folder (preferences, logs, token files): macOS
/// `~/Library/Application Support/WordCraft`, Windows `%APPDATA%\WordCraft`, otherwise
/// `$XDG_CONFIG_HOME/wordcraft` or `~/.config/wordcraft`.
/// A folder chosen by the app through [`use_settings_dir`] wins: it is a launch decision, not
/// an environment variable a child or a wrapper might have inherited. `WORDCRAFT_PROFILE_DIR`
/// overrides the entire folder for isolated runs otherwise.
pub fn settings_dir() -> Option<PathBuf> {
    if let Some(path) = CHOSEN.get() {
        return Some(path.clone());
    }
    if let Some(path) = std::env::var_os("WORDCRAFT_PROFILE_DIR").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    platform_settings_dir()
}

/// The platform's own folder, when nothing chose one (see [`settings_dir`]).
fn platform_settings_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/WordCraft"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("WordCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("wordcraft"))
    }
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

/// The token file `<settings>/control-key.<instance>`, with `<instance>` the Flatpak instance id
/// or `local-<port>`.
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
    fn a_chosen_folder_wins_over_the_environment() {
        // The chosen folder is process-wide and set once, so this test only reads the rule when
        // nothing chose one (the crate's own tests and the CLI never do).
        if CHOSEN.get().is_none() {
            assert_eq!(settings_dir(), super::platform_settings_dir());
            assert!(use_settings_dir(PathBuf::from("/chosen/profile")).is_ok());
            assert_eq!(settings_dir().as_deref(), Some(std::path::Path::new("/chosen/profile")));
            assert!(use_settings_dir(PathBuf::from("/other")).is_err(), "the folder is selected once");
        }
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
}
