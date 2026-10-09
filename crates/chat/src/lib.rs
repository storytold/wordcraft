//! Chat add-in: members, invites, keys, messages and rules. No UI and no network: the control
//! server and the egui pane both use the same [`hub::Hub`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod hub;
pub mod keys;
pub mod lang;
pub mod log;
pub mod rules;

pub use hub::{ChatError, Hub, Member, Message, Principal, Role};
pub use lang::{Lang, Text};

/// Pure parser for `/.flatpak-info`: the `instance-id=` value, only if it is `[A-Za-z0-9_-]+`
/// (it ends up in a file name, so anything with `/`, `.` or spaces is rejected).
pub fn parse_instance_id(info: &str) -> Option<String> {
    let v = info.lines().find_map(|l| l.strip_prefix("instance-id="))?.trim();
    (!v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then(|| v.to_string())
}

/// The Flatpak instance id, or `None` outside a sandbox (or when the value looks unsafe).
pub fn instance_id() -> Option<String> {
    parse_instance_id(&std::fs::read_to_string("/.flatpak-info").ok()?)
}

/// The directory that holds `ui.json` and the control key file. One rule for the app and the
/// MCP bridge: macOS `~/Library/Application Support/WordCraft`, Windows `%APPDATA%\WordCraft`,
/// otherwise `$XDG_CONFIG_HOME` or `~/.config`, plus `wordcraft`.
pub fn config_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
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

/// Key file of one app instance: `<config_dir>/control-key.<instance>`, where `config_dir` is the
/// directory that holds `ui.json` and `<instance>` is the Flatpak instance id or `local-<port>`.
/// The app and the MCP bridge both call this, so they agree on the path.
pub fn control_key_path(config_dir: &std::path::Path, port: u16) -> std::path::PathBuf {
    let inst = instance_id().unwrap_or_else(|| format!("local-{port}"));
    config_dir.join(format!("control-key.{inst}"))
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn parse_instance_id_accepts_only_safe_values() {
        assert_eq!(parse_instance_id("[Instance]\ninstance-id=abc_123-X\n").as_deref(), Some("abc_123-X"));
        assert_eq!(parse_instance_id("instance-id= 42 \n").as_deref(), Some("42"));
        assert_eq!(parse_instance_id("instance-id=../etc\n"), None);
        assert_eq!(parse_instance_id("instance-id=a/b\n"), None);
        assert_eq!(parse_instance_id("instance-id=..\n"), None);
        assert_eq!(parse_instance_id("instance-id=\n"), None);
        assert_eq!(parse_instance_id("other=1\n"), None);
    }

    #[test]
    fn language_from_its_code() {
        assert_eq!(Lang::parse("pt"), Lang::Pt);
        assert_eq!(Lang::parse(" PT "), Lang::Pt);
        assert_eq!(Lang::parse("en"), Lang::En);
        assert_eq!(Lang::parse("de"), Lang::En, "unknown: English");
        assert_eq!(Lang::default(), Lang::En);
        assert_eq!((Lang::En.code(), Lang::Pt.code()), ("en", "pt"));
    }

    #[test]
    fn instance_id_is_none_or_non_empty() {
        assert!(instance_id().is_none_or(|i| !i.is_empty()));
    }

    #[test]
    fn key_path_is_per_instance_in_the_config_dir() {
        let p = control_key_path(std::path::Path::new("/c/wordcraft"), 7981);
        assert_eq!(p.parent(), Some(std::path::Path::new("/c/wordcraft")));
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(name.starts_with("control-key."));
        assert!(name.len() > "control-key.".len());
        if instance_id().is_none() {
            assert_eq!(name, "control-key.local-7981");
        }
    }
}
