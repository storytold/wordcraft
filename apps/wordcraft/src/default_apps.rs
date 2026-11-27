//! File › Options › General › Make WordCraft the default for Word documents (#295).
//!
//! The installers register WordCraft for Word, RTF and ODT files without taking over the
//! defaults (`packaging/windows/wordcraft.wxs`, the `.desktop` file's `MimeType=`, `Info.plist`).
//! This is the button that makes it the default when the user asks; the work is
//! `craft::platform::default_app::make_default`, which this file gives WordCraft's identity:
//! - **Windows** doesn't let apps set defaults: it opens Settings › Default apps on WordCraft's
//!   registered page, where the user picks WordCraft per type.
//! - **Linux and BSD**: `xdg-mime default ai.storyteller.wordcraft.desktop <types>` for
//!   [`WORD_MIME_TYPES`], which writes the user's `mimeapps.list`. Only when the desktop entry is
//!   installed (deb, rpm, Flatpak; a bare AppImage has none), and not from inside the Flatpak
//!   sandbox, which can't change the host's defaults.
//! - **macOS** has no API for it here: the Options page explains Finder's Get Info › Change All.
//!
//! [`WORD_MIME_TYPES`]: self::WORD_MIME_TYPES

#[cfg(any(unix, windows, test))]
use craft::platform::default_app::AppIdentity;

/// The desktop entry the Linux packages install, and the Flatpak's app id.
#[cfg(any(unix, windows, test))]
const DESKTOP_ID: &str = "ai.storyteller.wordcraft.desktop";

/// The types the button makes WordCraft the default for on Linux and BSD: Word documents and
/// templates (also macro-enabled and legacy), RTF and ODT. The same set the Windows installer
/// lists under WordCraft's Capabilities; Markdown is only offered under Open With, and plain
/// text only on Windows and macOS (the `.desktop` file leaves it out, #4137).
/// Every one is in the `.desktop` file's `MimeType=` (a test checks).
#[cfg(any(unix, windows, test))]
const WORD_MIME_TYPES: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document", // .docx
    "application/vnd.ms-word.document.macroEnabled.12",                        // .docm
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template", // .dotx
    "application/vnd.ms-word.template.macroEnabled.12",                        // .dotm
    "application/msword",                                                      // .doc
    "application/msword-template",                                             // .dot
    "application/rtf",                                                         // .rtf
    "text/rtf",                                                                // .rtf (older name)
    "application/vnd.oasis.opendocument.text",                                 // .odt
];

/// Who WordCraft is to the system: the name the MSI registers under `RegisteredApplications`, the
/// desktop entry, and the types the button claims.
#[cfg(any(unix, windows, test))]
pub fn identity() -> AppIdentity {
    AppIdentity {
        registered_name: "WordCraft".to_string(),
        desktop_id: DESKTOP_ID.to_string(),
        mime_types: WORD_MIME_TYPES.iter().map(|t| (*t).to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every type the button claims is included in identity, and plain text/Markdown are never claimed.
    #[test]
    fn xdg_mime_sets_every_word_type_in_one_call() {
        let id = identity();
        assert_eq!(id.desktop_id, "ai.storyteller.wordcraft.desktop");
        assert_eq!(id.registered_name, "WordCraft");
        for t in [
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/msword",
            "application/rtf",
            "application/vnd.oasis.opendocument.text",
        ] {
            assert!(id.mime_types.iter().any(|a| a == t), "{t}");
        }
        assert!(!id.mime_types.iter().any(|a| a == "text/plain" || a == "text/markdown"), "plain text and Markdown are never claimed");
    }

    /// Every type the button claims is one the desktop entry declares, or desktops ignore it.
    #[test]
    fn the_desktop_entry_declares_every_claimed_type() {
        let entry = include_str!("../../../packaging/linux/ai.storyteller.wordcraft.desktop");
        let declared: Vec<&str> =
            entry.lines().find_map(|l| l.strip_prefix("MimeType=")).map(|v| v.split(';').filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        for t in WORD_MIME_TYPES {
            assert!(declared.contains(t), "{t} missing from MimeType=");
        }
        assert!(entry.lines().any(|l| l == "Exec=wordcraft %F"), "files are passed on the command line");
    }

    /// Declaring `text/plain` makes WordCraft a handler for every config, script and log on systems with no default set.
    #[test]
    fn the_desktop_entry_does_not_declare_plain_text() {
        let entry = include_str!("../../../packaging/linux/ai.storyteller.wordcraft.desktop");
        let declared: Vec<&str> =
            entry.lines().find_map(|l| l.strip_prefix("MimeType=")).map(|v| v.split(';').filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        assert!(!declared.is_empty(), "MimeType= line not found");
        assert!(!declared.contains(&"text/plain"), "text/plain must not be declared");
    }
}
