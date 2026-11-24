//! App-owned nightly profile fences; no stable fallback, migration or process environment changes.
//!
//! WordCraft keeps all of its user state in one settings folder (`wordcraft_control_key::settings_dir`):
//! `config.toml`, `window.json`, `logs/`, `recovery/`, `fonts/`, `Templates/`,
//! `custom-dictionary.txt` and the control token files. The suite's shared font store
//! (`Craft Fonts` / `craft-fonts`, which also holds installed Language packs) is the one other
//! per-user folder the app writes, and Nightly points it at the profile too
//! (`wordcraft_engine::packs::use_font_store_in`). Every stable location of either is fenced here.

use std::ffi::OsString;

use std::path::{Component, Path, PathBuf};

/// The app id of the nightly Flatpak this build ships in (`packaging/linux/flatpak-bundle.sh`).
pub const NIGHTLY_FLATPAK_ID: &str = "ai.storyteller.wordcraft.nightly";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    Linux,
    Mac,
}

impl Platform {
    fn current() -> Result<Self, String> {
        match std::env::consts::OS {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Mac),
            _ => Err("nightly-v1 is not defined for this operating system".into()),
        }
    }

    fn ignores_case(self) -> bool {
        self != Self::Linux
    }
}

fn absolute(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{label} must be absolute and contain no parent traversal"));
    }
    Ok(path)
}

fn required_base(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Result<PathBuf, String> {
    let value = env(key).filter(|v| !v.is_empty()).ok_or_else(|| format!("cannot establish the stable {key} boundary"))?;
    absolute(PathBuf::from(value), key)
}

fn xdg_base(env: &impl Fn(&str) -> Option<OsString>, key: &str, fallback: &str) -> Result<PathBuf, String> {
    match env(key).filter(|v| !v.is_empty()) {
        Some(value) => absolute(PathBuf::from(value), key),
        None => Ok(required_base(env, "HOME")?.join(fallback)),
    }
}

/// Resolve existing ancestors before creating anything. A dangling link or inaccessible ancestor
/// is not a missing directory that may be replaced or followed later.
fn canonical_future(path: &Path) -> Result<PathBuf, String> {
    let mut parent = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(parent) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(parent.file_name().ok_or("the profile root has no existing ancestor")?.to_os_string());
                parent = parent.parent().ok_or("the profile root has no existing ancestor")?;
            }
            Err(error) => return Err(format!("cannot inspect the profile boundary: {error}")),
        }
    }
    let mut resolved = std::fs::canonicalize(parent).map_err(|e| format!("cannot resolve the profile boundary: {e}"))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

fn overlaps(a: &Path, b: &Path, platform: Platform) -> bool {
    let parts = |p: &Path| {
        p.components()
            .map(|c| {
                let value = c.as_os_str().to_string_lossy().into_owned();
                if platform.ignores_case() { value.to_lowercase() } else { value }
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (parts(a), parts(b));
    a.starts_with(&b) || b.starts_with(&a)
}

fn installation_boundary(platform: Platform, exe: &Path) -> Result<PathBuf, String> {
    let parent = exe.parent().ok_or("cannot establish the installation boundary")?;
    if platform == Platform::Mac {
        // A profile in Resources is still inside the versioned install, not beside MacOS.
        if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))) {
            return Ok(bundle.to_path_buf());
        }
    }
    Ok(parent.to_path_buf())
}

fn forbidden_roots(platform: Platform, env: &impl Fn(&str) -> Option<OsString>, exe: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = match platform {
        Platform::Windows => {
            let mut roots = ["APPDATA", "LOCALAPPDATA"]
                .into_iter()
                .map(|key| required_base(env, key).map(|p| p.join("WordCraft")))
                .collect::<Result<Vec<_>, _>>()?;
            // The suite's shared font store and the MSI's default per-user and per-machine folders.
            roots.push(required_base(env, "LOCALAPPDATA")?.join("Craft Fonts"));
            roots.push(required_base(env, "LOCALAPPDATA")?.join("Programs").join("ArtCraft").join("WordCraft"));
            for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(base) = env(key).filter(|v| !v.is_empty()) {
                    roots.push(absolute(PathBuf::from(base), key)?.join("ArtCraft").join("WordCraft"));
                }
            }
            roots
        }
        Platform::Linux => {
            let mut roots =
                [("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share"), ("XDG_CACHE_HOME", ".cache"), ("XDG_STATE_HOME", ".local/state")]
                    .into_iter()
                    .map(|(key, fallback)| xdg_base(env, key, fallback).map(|p| p.join("wordcraft")))
                    .collect::<Result<Vec<_>, _>>()?;
            // The shared font store (fonts and Language packs) and the native packages' locations.
            roots.push(xdg_base(env, "XDG_DATA_HOME", ".local/share")?.join("craft-fonts"));
            roots.extend(["/usr/bin/wordcraft", "/usr/local/bin/wordcraft", "/opt/wordcraft"].map(PathBuf::from));
            // The stable Flatpak keeps its data in its own per-app folder.
            roots.push(required_base(env, "HOME")?.join(".var/app/ai.storyteller.wordcraft"));
            roots
        }
        Platform::Mac => {
            let home = required_base(env, "HOME")?;
            let library = home.join("Library");
            vec![
                library.join("Application Support/WordCraft"),
                library.join("Caches/WordCraft"),
                library.join("Caches/ai.storyteller.wordcraft"),
                library.join("Preferences/WordCraft"),
                library.join("Preferences/ai.storyteller.wordcraft.plist"),
                library.join("Saved Application State/ai.storyteller.wordcraft.savedState"),
                library.join("Application Support/Craft Fonts"),
                home.join("Applications/WordCraft.app"),
                PathBuf::from("/Applications/WordCraft.app"),
            ]
        }
    };
    roots.push(installation_boundary(platform, exe)?);
    roots.into_iter().map(|p| canonical_future(&p)).collect()
}

fn private_subdirectory(root: &Path, name: impl AsRef<Path>, label: &str) -> Result<PathBuf, String> {
    let chosen = canonical_future(&absolute(root.to_path_buf(), "the nightly profile root")?)?;
    let child = canonical_future(&chosen.join(name))?;
    if child == chosen || !child.starts_with(&chosen) {
        return Err(format!("the nightly {label} must stay inside its selected profile"));
    }
    Ok(child)
}

/// Every existing entry directly in the profile (`config.toml`, `logs`, `recovery`, `fonts`,
/// `Templates`, `window.json` and so on) must resolve inside it, so a link there cannot point
/// Nightly's writes at another app's data.
fn entries_stay_inside(root: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect the nightly profile: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot inspect the nightly profile: {e}"))?;
        private_subdirectory(root, entry.file_name(), "profile entry")?;
    }
    Ok(())
}

/// The app's exclusive create probe: a file only this process can have made, created and removed.
pub fn probe_writable(dir: &Path) -> std::io::Result<()> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let probe = dir.join(format!(".nightly-writable-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?;
    drop(file);
    std::fs::remove_file(probe)
}

/// The writable hook is injected so filesystem boundary tests exercise the real policy without
/// starting the app or creating the profile of the person running them.
pub fn validate(
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    validate_for(Platform::current()?, root, env, exe, writable)
}

fn validate_for(
    platform: Platform,
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    absolute(root.to_path_buf(), "the nightly profile root")?;
    // The developer switch that points a run at any folder: Nightly's folder comes from the Launcher.
    if env("WORDCRAFT_PROFILE_DIR").is_some_and(|v| !v.is_empty()) {
        return Err("nightly-v1 cannot be combined with WORDCRAFT_PROFILE_DIR".into());
    }
    // These rows are native tar/app-bundle launches, not a Flatpak package. Inheriting a Flatpak
    // runtime could point another profile at the stable app's token files; the nightly Flatpak
    // itself (`packaging/linux/flatpak-bundle.sh`) is the one sandbox this build ships in.
    if platform != Platform::Windows
        && let Some(id) = env("FLATPAK_ID").filter(|v| !v.is_empty())
        && id != NIGHTLY_FLATPAK_ID
    {
        return Err("nightly-v1 native packages cannot inherit a Flatpak instance; launch through the native Launcher".into());
    }
    let chosen = canonical_future(root)?;
    let exe = absolute(exe.ok_or("cannot establish the installation boundary")?.to_path_buf(), "the executable")?;
    let forbidden = forbidden_roots(platform, &env, &exe)?;
    if forbidden.iter().any(|stable| overlaps(&chosen, stable, platform)) {
        return Err("the nightly root must not overlap stable data or an installation".into());
    }
    // Existing aliases under the profile must not redirect its own subfolders elsewhere.
    entries_stay_inside(&chosen)?;
    // Recovery can read, adopt and sweep entries; a link there must never leave the profile.
    private_subdirectory(&chosen, "recovery", "recovery folder")?;
    let mut create = std::fs::DirBuilder::new();
    create.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        create.mode(0o700);
    }
    create.create(&chosen).map_err(|e| format!("cannot create the nightly profile: {e}"))?;
    let confirmed = std::fs::canonicalize(&chosen).map_err(|e| format!("cannot resolve the created nightly profile: {e}"))?;
    if confirmed != chosen || forbidden.iter().any(|stable| overlaps(&confirmed, stable, platform)) {
        return Err("the nightly profile changed while it was being validated".into());
    }
    private_subdirectory(&confirmed, "recovery", "recovery folder")?;
    entries_stay_inside(&confirmed)?;
    writable(&confirmed).map_err(|e| format!("the nightly profile is not writable: {e}"))?;
    Ok(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture {
        root: PathBuf,
        env: BTreeMap<String, OsString>,
        exe: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = loop {
                let candidate =
                    std::env::temp_dir().join(format!("wordcraft-profile-policy-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => break std::fs::canonicalize(candidate).unwrap(),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create owned fixture: {error}"),
                }
            };
            let home = root.join("home");
            let exe = root.join("install/wordcraft");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"fixture executable, never run").unwrap();
            let env = [("HOME", home), ("APPDATA", root.join("roaming")), ("LOCALAPPDATA", root.join("local"))]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into_os_string()))
                .collect();
            Self { root, env, exe }
        }

        fn get(&self, key: &str) -> Option<OsString> {
            self.env.get(key).cloned()
        }

        fn check(&self, platform: Platform, root: &Path) -> Result<PathBuf, String> {
            validate_for(
                platform,
                root,
                |key| self.get(key),
                Some(&self.exe),
                |dir| {
                    let probe = dir.join("exclusive-test-probe");
                    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?;
                    drop(file);
                    std::fs::remove_file(probe)
                },
            )
        }

        fn home(&self) -> PathBuf {
            PathBuf::from(self.get("HOME").unwrap())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only the directory successfully created by this fixture is owned for removal.
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn linux_fences_the_settings_folders_font_store_and_packages_but_allows_the_launcher_sibling() {
        let mut fixture = Fixture::new();
        for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"] {
            fixture.env.insert(key.into(), fixture.root.join(key).into_os_string());
        }
        for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"] {
            let stable = PathBuf::from(fixture.get(key).unwrap()).join("wordcraft");
            std::fs::create_dir_all(&stable).unwrap();
            std::fs::write(stable.join("stable-sentinel"), b"stable bytes").unwrap();
            for candidate in [stable.clone(), stable.join("nightly"), stable.parent().unwrap().to_path_buf()] {
                assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("overlap"));
            }
            assert_eq!(std::fs::read(stable.join("stable-sentinel")).unwrap(), b"stable bytes");
        }
        let fonts = PathBuf::from(fixture.get("XDG_DATA_HOME").unwrap()).join("craft-fonts");
        std::fs::create_dir_all(fonts.join("language-packs")).unwrap();
        for candidate in [fonts.clone(), fonts.join("nightly")] {
            assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("overlap"));
        }
        for package in ["/usr/bin/wordcraft", "/usr/local/bin/wordcraft", "/opt/wordcraft"] {
            assert!(fixture.check(Platform::Linux, &PathBuf::from(package)).is_err());
        }
        let data = PathBuf::from(fixture.get("XDG_DATA_HOME").unwrap());
        let nightly = data.join("ArtCraftLauncher/nightly/profiles/wordcraft");
        assert_eq!(fixture.check(Platform::Linux, &nightly).unwrap(), nightly);
        assert_eq!(std::fs::read_dir(nightly).unwrap().count(), 0);
    }

    #[test]
    fn linux_fences_the_stable_flatpak_data_folder() {
        let fixture = Fixture::new();
        let flatpak = fixture.home().join(".var/app/ai.storyteller.wordcraft");
        for candidate in [flatpak.clone(), flatpak.join("data/nightly")] {
            assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("overlap"));
        }
        assert!(!flatpak.exists());
        assert!(fixture.check(Platform::Linux, &fixture.home().join(".var/app/ai.storyteller.launcher/nightly")).is_ok());
        // Inside the Nightly Flatpak, XDG_DATA_HOME is this app's own persistent data folder, so
        // the launcher's profile lands there rather than on the discarded sandbox home.
        let nightly = fixture.home().join(".var/app/ai.storyteller.wordcraft.nightly/data/wordcraft-nightly");
        assert_eq!(fixture.check(Platform::Linux, &nightly).unwrap(), nightly);
    }

    #[test]
    fn mac_fences_the_app_library_subtrees_the_shared_font_store_and_the_whole_bundle() {
        let mut fixture = Fixture::new();
        fixture.exe = fixture.root.join("install/WordCraft Nightly.app/Contents/MacOS/WordCraft");
        std::fs::create_dir_all(fixture.exe.parent().unwrap()).unwrap();
        std::fs::write(&fixture.exe, b"fixture executable, never run").unwrap();
        let library = fixture.home().join("Library");
        for suffix in [
            "Application Support/WordCraft",
            "Caches/WordCraft",
            "Caches/ai.storyteller.wordcraft",
            "Preferences/WordCraft",
            "Saved Application State/ai.storyteller.wordcraft.savedState",
            "Application Support/Craft Fonts",
        ] {
            let stable = library.join(suffix);
            assert!(fixture.check(Platform::Mac, &stable.join("nightly")).unwrap_err().contains("overlap"));
            assert!(!stable.exists());
        }
        let inside_resources = fixture.exe.parent().unwrap().parent().unwrap().join("Resources/profile");
        assert!(fixture.check(Platform::Mac, &inside_resources).unwrap_err().contains("overlap"));
        assert!(!inside_resources.exists());
        let valid = library.join("Application Support/ArtCraftLauncher/nightly/profiles/wordcraft");
        assert_eq!(fixture.check(Platform::Mac, &valid).unwrap(), valid);
    }

    #[test]
    fn windows_roaming_local_font_store_and_install_fences_remain_active() {
        let fixture = Fixture::new();
        let mut fixture = fixture;
        for key in ["APPDATA", "LOCALAPPDATA"] {
            let stable = PathBuf::from(fixture.get(key).unwrap()).join("WordCraft");
            assert!(fixture.check(Platform::Windows, &stable.join("nightly")).unwrap_err().contains("overlap"));
            assert!(!stable.exists());
        }
        let fonts = PathBuf::from(fixture.get("LOCALAPPDATA").unwrap()).join("Craft Fonts");
        assert!(fixture.check(Platform::Windows, &fonts.join("nightly")).unwrap_err().contains("overlap"));
        assert!(fixture.check(Platform::Windows, &fixture.exe.parent().unwrap().join("profile")).is_err());
        let local = PathBuf::from(fixture.get("LOCALAPPDATA").unwrap());
        let per_user = local.join("Programs").join("ArtCraft").join("WordCraft");
        assert!(fixture.check(Platform::Windows, &per_user.join("nightly")).unwrap_err().contains("overlap"));
        for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            let base = fixture.root.join(format!("machine {key}"));
            fixture.env.insert(key.into(), base.clone().into_os_string());
            let machine = base.join("ArtCraft").join("WordCraft");
            assert!(fixture.check(Platform::Windows, &machine.join("nightly")).unwrap_err().contains("overlap"));
            assert!(!machine.exists());
        }
        let sibling = local.join("Programs").join("ArtCraft").join("ArtCraftLauncher").join("nightly").join("wordcraft");
        assert!(fixture.check(Platform::Windows, &sibling).is_ok(), "the Launcher's own Programs sibling is not WordCraft's install");
    }

    #[test]
    fn overlap_check_handles_ancestor_descendant_and_prefix_lookalikes() {
        let fixture = Fixture::new();
        let root = fixture.home().join("nightly/profiles/wordcraft");
        for platform in [Platform::Windows, Platform::Linux, Platform::Mac] {
            assert!(overlaps(&root, &root.join("recovery"), platform));
            assert!(overlaps(&root.join("recovery"), &root, platform));
            assert!(!overlaps(&root, &root.with_file_name("wordcraft-other"), platform));
        }
    }

    #[cfg(unix)]
    #[test]
    fn recovery_and_entry_aliases_cannot_leave_the_profile_and_stable_bytes_stay_untouched() {
        let fixture = Fixture::new();
        let stable = fixture.home().join(".config/wordcraft/recovery");
        std::fs::create_dir_all(&stable).unwrap();
        std::fs::write(stable.join("stable-autosave"), b"stable recovery bytes").unwrap();
        for (index, target) in [stable.clone(), fixture.root.join("missing recovery target")].into_iter().enumerate() {
            let private = fixture.root.join(format!("nightly-{index}"));
            std::fs::create_dir(&private).unwrap();
            std::os::unix::fs::symlink(&target, private.join("recovery")).unwrap();
            let probed = std::cell::Cell::new(false);
            let result = validate_for(
                Platform::Linux,
                &private,
                |key| fixture.get(key),
                Some(&fixture.exe),
                |_| {
                    probed.set(true);
                    Ok(())
                },
            );
            assert!(result.is_err(), "recovery must not escape the private profile or follow a dangling alias");
            assert!(!probed.get());
        }
        for name in ["logs", "fonts", "config.toml"] {
            let private = fixture.root.join(format!("nightly-entry-{name}"));
            std::fs::create_dir(&private).unwrap();
            std::os::unix::fs::symlink(&stable, private.join(name)).unwrap();
            assert!(fixture.check(Platform::Linux, &private).is_err(), "{name} must not lead outside the profile");
        }
        assert_eq!(std::fs::read(stable.join("stable-autosave")).unwrap(), b"stable recovery bytes");
        assert_eq!(std::fs::read_dir(stable).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_recovery_alias_inside_the_profile_keeps_it_usable() {
        let fixture = Fixture::new();
        let private = fixture.root.join("nightly-private");
        let recovery = private.join("private-recovery");
        std::fs::create_dir_all(&recovery).unwrap();
        std::os::unix::fs::symlink(&recovery, private.join("recovery")).unwrap();
        assert!(fixture.check(Platform::Linux, &private).is_ok());
        assert_eq!(std::fs::canonicalize(private.join("recovery")).unwrap(), recovery);
    }

    #[test]
    fn ambiguous_environment_and_non_native_sandbox_are_refused_before_creation() {
        let mut fixture = Fixture::new();
        let candidate = fixture.home().join("nightly/profiles/wordcraft");
        fixture.env.insert("XDG_CACHE_HOME".into(), "relative/cache".into());
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("absolute"));
        fixture.env.remove("XDG_CACHE_HOME");
        fixture.env.insert("WORDCRAFT_PROFILE_DIR".into(), fixture.home().join("custom-stable").into_os_string());
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("WORDCRAFT_PROFILE_DIR"));
        fixture.env.remove("WORDCRAFT_PROFILE_DIR");
        fixture.env.insert("FLATPAK_ID".into(), "ai.storyteller.wordcraft".into());
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("Flatpak"));
        fixture.env.remove("FLATPAK_ID");
        // The nightly Flatpak is the one sandbox this build ships in, so its own id is accepted.
        let flatpak_profile = fixture.home().join("nightly/flatpak-profile");
        fixture.env.insert("FLATPAK_ID".into(), NIGHTLY_FLATPAK_ID.into());
        assert_eq!(fixture.check(Platform::Linux, &flatpak_profile).unwrap(), flatpak_profile);
        fixture.env.remove("FLATPAK_ID");
        fixture.env.remove("HOME");
        assert!(fixture.check(Platform::Linux, &candidate).unwrap_err().contains("HOME"));
        assert!(!candidate.exists());
    }

    #[test]
    fn non_directory_and_failed_write_probe_never_fall_back_to_stable() {
        let fixture = Fixture::new();
        let file = fixture.home().join("not-a-directory");
        std::fs::write(&file, b"do not replace me").unwrap();
        assert!(fixture.check(Platform::Linux, &file).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"do not replace me");
        let candidate = fixture.home().join("nightly");
        let calls = std::cell::Cell::new(0);
        let result = validate_for(
            Platform::Linux,
            &candidate,
            |key| fixture.get(key),
            Some(&fixture.exe),
            |_| {
                calls.set(calls.get() + 1);
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected write refusal"))
            },
        );
        assert!(result.unwrap_err().contains("not writable"));
        assert_eq!(calls.get(), 1);
        assert!(!fixture.home().join(".config/wordcraft").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_dangling_aliases_cannot_create_or_modify_stable_data() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let stable = fixture.home().join(".config/wordcraft");
        std::fs::create_dir_all(&stable).unwrap();
        std::fs::write(stable.join("sentinel"), b"untouched").unwrap();
        let alias = fixture.home().join("alias");
        symlink(&stable, &alias).unwrap();
        assert!(fixture.check(Platform::Linux, &alias.join("nightly")).unwrap_err().contains("overlap"));
        let missing = fixture.home().join("missing-target");
        let dangling = fixture.home().join("dangling");
        symlink(&missing, &dangling).unwrap();
        assert!(fixture.check(Platform::Linux, &dangling.join("nightly")).is_err());
        assert!(!missing.exists());
        assert!(!stable.join("nightly").exists());
        assert_eq!(std::fs::read(stable.join("sentinel")).unwrap(), b"untouched");
    }
}
