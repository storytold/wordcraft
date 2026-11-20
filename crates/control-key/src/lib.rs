//! The control channel's key file, one rule for the app (which writes it) and the MCP bridge
//! (which reads it): `<settings>/control-key.<instance>`. See `docs/control-protocol.md`, Keys.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};

/// The app's settings folder (preferences, logs, key files): macOS
/// `~/Library/Application Support/WordCraft`, Windows `%APPDATA%\WordCraft`, otherwise
/// `$XDG_CONFIG_HOME/wordcraft` or `~/.config/wordcraft`.
/// `WORDCRAFT_PROFILE_DIR` overrides the entire folder for isolated runs.
pub fn settings_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WORDCRAFT_PROFILE_DIR").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    // An empty base path would put settings, logs and control keys in the current directory.
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join("Library/Application Support/WordCraft"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").filter(|a| !a.is_empty()).map(|a| PathBuf::from(a).join("WordCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|c| !c.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".config")))
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

    #[test]
    fn settings_dir_is_the_apps_folder() {
        if let Some(d) = settings_dir() {
            let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
            assert!(name == "wordcraft" || name == "WordCraft", "{}", d.display());
        }
    }

    #[test]
    fn settings_dir_ignores_empty_environment_paths() {
        const CHILD: &str = "WORDCRAFT_SETTINGS_DIR_TEST_CHILD";
        const EXPECTED: &str = "WORDCRAFT_SETTINGS_DIR_TEST_EXPECTED";
        if std::env::var_os(CHILD).is_some() {
            assert_eq!(settings_dir(), std::env::var_os(EXPECTED).map(PathBuf::from));
            return;
        }

        // Change only a child process's environment: other tests may read these variables.
        let check = |variables: &[(&str, &Path)], expected: Option<&Path>| {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "tests::settings_dir_ignores_empty_environment_paths"]);
            child.env(CHILD, "1").env_remove(EXPECTED);
            for name in ["WORDCRAFT_PROFILE_DIR", "HOME", "APPDATA", "XDG_CONFIG_HOME"] {
                child.env(name, "");
            }
            for (name, value) in variables {
                child.env(name, value);
            }
            if let Some(path) = expected {
                child.env(EXPECTED, path);
            }
            let output = child.output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
            assert!(stdout.contains("1 passed"), "{stdout}");
        };

        let root = std::env::current_dir().unwrap().join("settings-test-root");
        let profile = root.join("isolated-profile");
        let (base, expected) = if cfg!(target_os = "macos") {
            ("HOME", root.join("Library/Application Support/WordCraft"))
        } else if cfg!(windows) {
            ("APPDATA", root.join("WordCraft"))
        } else {
            ("HOME", root.join(".config/wordcraft"))
        };
        check(&[], None);
        check(&[(base, &root)], Some(&expected));
        check(&[(base, &root), ("WORDCRAFT_PROFILE_DIR", &profile)], Some(&profile));
        if !cfg!(target_os = "macos") && !cfg!(windows) {
            let xdg = root.join("xdg");
            check(&[("HOME", &root), ("XDG_CONFIG_HOME", &xdg)], Some(&xdg.join("wordcraft")));
        }
    }
}
