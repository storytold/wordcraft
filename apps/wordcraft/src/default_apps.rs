//! File › Options › General › Make WordCraft the default for Word documents (#295).
//!
//! The installers register WordCraft for Word, RTF and ODT files without taking over the
//! defaults (`packaging/windows/wordcraft.wxs`, the `.desktop` file's `MimeType=`, `Info.plist`).
//! This is the button that makes it the default when the user asks:
//! - **Windows** doesn't let apps set defaults: it opens Settings › Default apps on WordCraft's
//!   page (`ms-settings:defaultapps?registeredAppMachine=WordCraft`; Windows 10 ignores the query
//!   and shows the Default apps list), where the user picks WordCraft per type.
//! - **Linux and BSD**: `xdg-mime default ai.storyteller.wordcraft.desktop <types>` for
//!   [`WORD_MIME_TYPES`], which writes the user's `mimeapps.list`. Only when the desktop entry is
//!   installed (deb, rpm, Flatpak; a bare AppImage has none), and not from inside the Flatpak
//!   sandbox, which can't change the host's defaults.
//! - **macOS** has no API for it here: the Options page explains Finder's Get Info › Change All.
//!
//! [`WORD_MIME_TYPES`]: self::WORD_MIME_TYPES

use wordcraft_ui_egui::DefaultApp;

/// The `Services::make_default_app` hook.
pub type Hook = Box<dyn Fn() -> Result<DefaultApp, String>>;

/// The hook for this platform; `None` where there is no button (macOS).
pub fn hook() -> Option<Hook> {
    #[cfg(target_os = "windows")]
    let hook: Option<Hook> = Some(Box::new(windows::open_default_apps));
    #[cfg(all(unix, not(target_os = "macos")))]
    let hook: Option<Hook> = Some(Box::new(xdg::make_default));
    #[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos")))))]
    let hook: Option<Hook> = None;
    hook
}

/// The desktop entry the Linux packages install, and the Flatpak's app id.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
const DESKTOP_ID: &str = "ai.storyteller.wordcraft.desktop";

/// The types the button makes WordCraft the default for on Linux and BSD: Word documents and
/// templates (also macro-enabled and legacy), RTF and ODT. The same set the Windows installer
/// lists under WordCraft's Capabilities; Markdown is only offered under Open With, and plain
/// text only on Windows and macOS (the `.desktop` file leaves it out, #4137).
/// Every one is in the `.desktop` file's `MimeType=` (a test checks).
#[cfg(any(all(unix, not(target_os = "macos")), test))]
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

/// The arguments for `xdg-mime` that make `desktop_id` the default for [`WORD_MIME_TYPES`]: one
/// call sets them all.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn xdg_mime_args(desktop_id: &str) -> Vec<String> {
    ["default", desktop_id].into_iter().chain(WORD_MIME_TYPES.iter().copied()).map(str::to_string).collect()
}

/// The `applications` folders the desktop looks for entries in, from `XDG_DATA_HOME` (default
/// `~/.local/share`) and `XDG_DATA_DIRS` (default `/usr/local/share:/usr/share`). Relative paths
/// are ignored, as the XDG Base Directory spec says.
#[cfg(all(unix, any(not(target_os = "macos"), test)))]
fn application_dirs(data_home: Option<&str>, data_dirs: Option<&str>, home: Option<&str>) -> Vec<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let absolute = |s: &&str| Path::new(s).is_absolute();
    let home_dir = data_home
        .filter(|s| !s.is_empty())
        .filter(absolute)
        .map(PathBuf::from)
        .or_else(|| home.filter(|s| !s.is_empty()).filter(absolute).map(|h| Path::new(h).join(".local/share")));
    let system = data_dirs.filter(|s| !s.is_empty()).unwrap_or("/usr/local/share:/usr/share");
    home_dir
        .into_iter()
        .chain(system.split(':').filter(|s| !s.is_empty()).filter(absolute).map(PathBuf::from))
        .map(|d| d.join("applications"))
        .collect()
}

#[cfg(target_os = "windows")]
mod windows {
    use wordcraft_ui_egui::DefaultApp;

    /// Settings › Default apps, on the page of the app registered as `WordCraft` under
    /// `HKLM\Software\RegisteredApplications` (the MSI writes it).
    const SETTINGS_URI: &str = "ms-settings:defaultapps?registeredAppMachine=WordCraft";

    pub(super) fn open_default_apps() -> Result<DefaultApp, String> {
        open::that_detached(SETTINGS_URI).map_err(|e| format!("couldn't open Settings: {e}"))?;
        Ok(DefaultApp::SettingsOpened)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod xdg {
    use std::process::Command;

    use wordcraft_ui_egui::DefaultApp;

    use super::{DESKTOP_ID, application_dirs, xdg_mime_args};

    /// The longest error text from `xdg-mime` shown in the status bar.
    const MAX_ERROR: usize = 200;

    pub(super) fn make_default() -> Result<DefaultApp, String> {
        if std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists() {
            return Err(
                "the Flatpak sandbox can't change default apps; choose WordCraft under Open With in your file manager or system settings".into()
            );
        }
        let var = |k: &str| std::env::var(k).ok();
        let dirs = application_dirs(var("XDG_DATA_HOME").as_deref(), var("XDG_DATA_DIRS").as_deref(), var("HOME").as_deref());
        if !dirs.iter().any(|d| d.join(DESKTOP_ID).is_file()) {
            return Err(format!("{DESKTOP_ID} isn't installed (an AppImage needs desktop integration first)"));
        }
        let out = Command::new("xdg-mime").args(xdg_mime_args(DESKTOP_ID)).output().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "xdg-mime isn't installed (it comes with xdg-utils)".to_string(),
            _ => format!("couldn't run xdg-mime: {e}"),
        })?;
        if out.status.success() {
            return Ok(DefaultApp::MadeDefault);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        let message: String = first.chars().take(MAX_ERROR).collect();
        Err(if message.is_empty() { format!("xdg-mime failed ({})", out.status) } else { format!("xdg-mime failed: {message}") })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn xdg_mime_sets_every_word_type_in_one_call() {
        let args = xdg_mime_args(DESKTOP_ID);
        assert_eq!(args.first().map(String::as_str), Some("default"));
        assert_eq!(args.get(1).map(String::as_str), Some("ai.storyteller.wordcraft.desktop"));
        assert_eq!(&args[2..], WORD_MIME_TYPES);
        for t in [
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/msword",
            "application/rtf",
            "application/vnd.oasis.opendocument.text",
        ] {
            assert!(args.iter().any(|a| a == t), "{t}");
        }
        assert!(!args.iter().any(|a| a == "text/plain" || a == "text/markdown"), "plain text and Markdown are never claimed");
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

    /// The macOS type of each document extension File › Open offers ([`OPEN_EXTS`]): Finder
    /// offers WordCraft for a file, and Change All can make it the default, only when the
    /// bundle's `CFBundleDocumentTypes` claims the file's type (#4983).
    ///
    /// [`OPEN_EXTS`]: wordcraft_engine::io::OPEN_EXTS
    const MAC_TYPES: &[(&str, &str)] = &[
        ("docx", "org.openxmlformats.wordprocessingml.document"),
        ("docm", "org.openxmlformats.wordprocessingml.document.macroenabled"),
        ("dotx", "org.openxmlformats.wordprocessingml.template"),
        ("dotm", "org.openxmlformats.wordprocessingml.template.macroenabled"),
        ("doc", "com.microsoft.word.doc"),
        ("dot", "com.microsoft.word.dot"),
        ("odt", "org.oasis-open.opendocument.text"),
        ("rtf", "public.rtf"),
        ("md", "net.daringfireball.markdown"),
        ("markdown", "net.daringfireball.markdown"),
        ("html", "public.html"),
        ("htm", "public.html"),
        ("txt", "public.plain-text"),
    ];

    /// Other identifiers apps declare for an extension in [`MAC_TYPES`], claimed too: Pages
    /// declares the macro-enabled types with a hyphen, and on a Mac where its declaration wins a
    /// `.docm` or `.dotm` file gets that identifier. They aren't imported; the system's own
    /// declaration stays the one WordCraft relies on.
    const MAC_ALIASES: &[(&str, &str)] = &[
        ("docm", "org.openxmlformats.wordprocessingml.document.macro-enabled"),
        ("dotm", "org.openxmlformats.wordprocessingml.template.macro-enabled"),
    ];

    /// Formats WordCraft opens that the bundle deliberately doesn't claim: macOS has no system
    /// type for LaTeX (TeX editors declare their own), and claiming XML or JSON would put
    /// WordCraft under Open With for every data file (WordCraft's own `.wcraft.json` is opened
    /// from File › Open).
    const MAC_UNCLAIMED: &[&str] = &["tex", "latex", "ltx", "xml", "wcraft.json", "json"];

    fn info_plist() -> plist::Dictionary {
        let source = include_bytes!("../../../packaging/macos/Info.plist.in");
        plist::from_bytes(source).unwrap()
    }

    fn strings(v: Option<&plist::Value>) -> Vec<String> {
        v.and_then(plist::Value::as_array).map(|a| a.iter().filter_map(|s| s.as_string().map(str::to_string)).collect()).unwrap_or_default()
    }

    /// Every type the bundle claims, from every `CFBundleDocumentTypes` entry, with that entry's
    /// role and rank.
    fn claimed_types(plist: &plist::Dictionary) -> Vec<(String, String, String)> {
        let entries = plist.get("CFBundleDocumentTypes").and_then(plist::Value::as_array).unwrap();
        let mut out = Vec::new();
        for e in entries {
            let e = e.as_dictionary().unwrap();
            let text = |k: &str| e.get(k).and_then(plist::Value::as_string).unwrap_or("").to_string();
            for t in strings(e.get("LSItemContentTypes")) {
                out.push((t, text("CFBundleTypeRole"), text("LSHandlerRank")));
            }
        }
        out
    }

    #[test]
    fn every_format_wordcraft_opens_has_a_mac_type() {
        for ext in wordcraft_engine::io::OPEN_EXTS {
            assert!(
                MAC_TYPES.iter().any(|(e, _)| e == ext) || MAC_UNCLAIMED.contains(ext),
                ".{ext} opens in WordCraft: give it its macOS type in MAC_TYPES (and Info.plist.in) or list it in MAC_UNCLAIMED"
            );
        }
        for (ext, _) in MAC_TYPES {
            assert!(wordcraft_engine::io::OPEN_EXTS.contains(ext), ".{ext} is claimed on macOS but WordCraft doesn't open it");
        }
    }

    /// #4983: WordCraft can be chosen in Finder (Open With, Get Info › Change All) for every
    /// document it opens, as an editor, without taking a default by itself.
    #[test]
    fn the_mac_bundle_claims_every_document_type_it_opens() {
        let plist = info_plist();
        let claims = claimed_types(&plist);
        for (ext, uti) in MAC_TYPES {
            let Some((_, role, rank)) = claims.iter().find(|(t, _, _)| t == uti) else {
                panic!(".{ext} ({uti}) is missing from CFBundleDocumentTypes in packaging/macos/Info.plist.in");
            };
            assert_eq!(role, "Editor", "{uti}: WordCraft edits and saves it");
            assert_eq!(rank, "Alternate", "{uti}: never take a default over another app by itself");
        }
        for (ext, uti) in MAC_ALIASES {
            assert!(MAC_TYPES.iter().any(|(e, _)| e == ext), "{uti}: .{ext} isn't a format WordCraft claims");
            let Some((_, role, rank)) = claims.iter().find(|(t, _, _)| t == uti) else {
                panic!(".{ext} ({uti}, as Pages declares it) is missing from CFBundleDocumentTypes");
            };
            assert_eq!((role.as_str(), rank.as_str()), ("Editor", "Alternate"), "{uti}");
        }
        for (uti, _, _) in &claims {
            assert!(MAC_TYPES.iter().chain(MAC_ALIASES).any(|(_, t)| t == uti), "{uti} is claimed but no format WordCraft opens has it");
        }
    }

    /// A claimed type that isn't one of Apple's `public.*` types is declared again (imported), with
    /// the extensions WordCraft opens, so a Mac that doesn't know it (an older macOS, no Word)
    /// still gives the files that type rather than a dynamic one no claim matches.
    #[test]
    fn the_mac_bundle_imports_the_types_macos_may_not_know() {
        let plist = info_plist();
        let imported = plist.get("UTImportedTypeDeclarations").and_then(plist::Value::as_array).unwrap();
        assert!(plist.get("UTExportedTypeDeclarations").is_none(), "WordCraft owns none of these types: import, never export");
        for (ext, uti) in MAC_TYPES.iter().filter(|(_, t)| !t.starts_with("public.")) {
            let decl = imported
                .iter()
                .filter_map(plist::Value::as_dictionary)
                .find(|d| d.get("UTTypeIdentifier").and_then(plist::Value::as_string) == Some(uti));
            let Some(decl) = decl else { panic!("{uti} (.{ext}) has no UTImportedTypeDeclarations entry") };
            let tags = decl.get("UTTypeTagSpecification").and_then(plist::Value::as_dictionary).unwrap();
            assert!(strings(tags.get("public.filename-extension")).iter().any(|e| e == ext), "{uti} doesn't list .{ext}");
            assert!(!strings(decl.get("UTTypeConformsTo")).is_empty(), "{uti} conforms to nothing");
        }
    }

    #[cfg(unix)]
    #[test]
    fn application_dirs_follow_the_xdg_spec() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            application_dirs(None, None, Some("/home/a")),
            [p("/home/a/.local/share/applications"), p("/usr/local/share/applications"), p("/usr/share/applications")]
        );
        assert_eq!(
            application_dirs(Some("/data"), Some("/opt/share::relative:/usr/share"), Some("/home/a")),
            [p("/data/applications"), p("/opt/share/applications"), p("/usr/share/applications")]
        );
        // Empty or relative values fall back to the defaults; no home at all is fine.
        assert_eq!(application_dirs(Some(""), Some(""), None), [p("/usr/local/share/applications"), p("/usr/share/applications")]);
        assert_eq!(application_dirs(Some("rel"), None, Some("also-rel")), [p("/usr/local/share/applications"), p("/usr/share/applications")]);
    }
}
