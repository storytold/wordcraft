//! Desktop update check and one-click install (#15, #201).
//!
//! Logic only: HTTP and installers are injected through [`Session::update_http`] and
//! [`Session::update_install`]. Parses GitHub's `releases/latest` JSON defensively; verifies
//! downloads against `SHA256SUMS.txt` on the release.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// GitHub API for the latest published WordCraft release.
pub const LATEST_API: &str = "https://api.github.com/repos/storytold/wordcraft/releases/latest";
/// Every browser link the UI opens stays under this prefix.
pub const RELEASES_PAGE: &str = "https://github.com/storytold/wordcraft/releases/";
pub const CHECKSUMS_NAME: &str = "SHA256SUMS.txt";
/// At most one automatic check per day.
pub const INTERVAL_SECS: u64 = 24 * 60 * 60;
/// Release JSON and checksum files are capped (hostile or mistaken responses).
pub const MAX_JSON: usize = 256 * 1024;
/// Download size cap (largest release asset today is well below this).
pub const MAX_DOWNLOAD: usize = 512 * 1024 * 1024;

pub type Version = (u64, u64, u64);

/// Catalogued English messages for update failures. The interface translates these by key; the
/// technical cause rides along in [`UpdateError::detail`] and is only logged, never shown raw.
pub const MSG_CHECK_FAILED: &str = "The update check could not reach GitHub. Check your connection and try again.";
pub const MSG_BAD_RELEASE: &str = "GitHub returned an update WordCraft could not read.";
pub const MSG_NO_INSTALLER: &str = "This release has no installer for your system.";
pub const MSG_NO_CHECKSUMS: &str = "The release does not publish SHA-256 checksums, so the download was not installed.";
pub const MSG_CHECKSUMS_UNREADABLE: &str = "The release's checksum file could not be read.";
pub const MSG_CHECKSUM_MISSING: &str = "The release does not list a checksum for this file.";
pub const MSG_CHECKSUM_MISMATCH: &str = "The downloaded file does not match the published SHA-256 checksum.";
pub const MSG_NO_INSTALL_FN: &str = "The update installer is not available in this build.";
pub const MSG_APPLY_FAILED: &str = "The update could not be installed.";
pub const MSG_START_FAILED: &str = "The update could not be started.";

/// An update failure: a stable, catalogued message plus the underlying technical detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateError {
    pub message: &'static str,
    pub detail: String,
}

impl UpdateError {
    pub fn new(message: &'static str, detail: impl Into<String>) -> Self {
        Self { message, detail: detail.into() }
    }
}

/// Persisted update settings (saved in `ui.json` with the rest of the UI state).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatePrefs {
    /// File › Options › Check for updates on startup (off by default).
    pub check_on_startup: bool,
    /// Whether the first-run prompt has been shown.
    pub asked_startup: bool,
    /// Unix seconds of the last check attempt (success or failure).
    pub last_check_unix: u64,
    /// Version text the user chose to skip (`major.minor.patch`).
    pub skipped_version: String,
}

/// How this copy of WordCraft was installed (drives which release asset to fetch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    WindowsMsi,
    WindowsPortableZip,
    MacDmg,
    MacApp,
    LinuxDeb,
    Flatpak,
    AppImage,
    Unknown,
}

impl InstallKind {
    /// Self-updaters: show the notice only; updates use their own mechanism (#22).
    pub fn external_updater(self) -> bool {
        matches!(self, InstallKind::Flatpak | InstallKind::AppImage)
    }

    /// One-click install (`msiexec`, `pkexec dpkg`) — the app quits after a verified handoff.
    pub fn supports_one_click(self) -> bool {
        matches!(self, InstallKind::WindowsMsi | InstallKind::LinuxDeb)
    }

    /// **Update** downloads a release asset, verifies SHA-256, then hands off (installer, or open `.dmg` on macOS).
    pub fn offers_verified_update(self) -> bool {
        matches!(self, InstallKind::WindowsMsi | InstallKind::LinuxDeb | InstallKind::MacApp | InstallKind::MacDmg)
    }
}

/// Detect install kind from the running executable path and environment (explicit, tested).
pub fn detect_install_kind(exe: &str, env: &dyn EnvRead) -> InstallKind {
    if env.flatpak() {
        return InstallKind::Flatpak;
    }
    if env.appimage().is_some() {
        return InstallKind::AppImage;
    }
    if let Some(kind) = env.install_override() {
        return kind;
    }
    let normalized = exe.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if env.windows() {
        // MSI vs portable is decided only by installer evidence (`App Paths` registry entry), not
        // by folder names. Portable copies and custom install paths never get that entry.
        return if env.windows_msi_installed(exe) { InstallKind::WindowsMsi } else { InstallKind::Unknown };
    }
    if env.macos() {
        if mac_app_bundle_in_system_applications(&lower) {
            return InstallKind::MacApp;
        }
        return InstallKind::Unknown;
    }
    if env.linux() {
        if linux_system_install_path(&lower) {
            if env.linux_deb_package(exe) {
                return InstallKind::LinuxDeb;
            }
            return InstallKind::Unknown;
        }
        return InstallKind::Unknown;
    }
    InstallKind::Unknown
}

fn linux_system_install_path(lower_exe: &str) -> bool {
    lower_exe.starts_with("/usr/bin/") || lower_exe.starts_with("/usr/local/bin/") || lower_exe.contains("/opt/wordcraft/")
}

/// macOS installs only get a verified download from the system `/Applications` bundle; a copy in
/// `~/Applications` or elsewhere uses the release page. The `.dmg` is opened in Finder for a
/// manual drag-install (no bundle replacement, no privilege escalation).
fn mac_app_bundle_in_system_applications(lower_exe: &str) -> bool {
    lower_exe.starts_with("/applications/") && lower_exe.contains(".app/contents/macos/")
}

/// Environment hooks for install detection (real env or test fake).
pub trait EnvRead {
    fn flatpak(&self) -> bool;
    fn appimage(&self) -> Option<String>;
    fn install_override(&self) -> Option<InstallKind>;
    fn windows(&self) -> bool;
    fn macos(&self) -> bool;
    fn linux(&self) -> bool;
    /// True when `dpkg-query -S` reports the installed binary (`.deb` installs only).
    fn linux_deb_package(&self, exe: &str) -> bool;
    /// True when `exe` is the executable a Windows MSI installed, judged by the installer's own
    /// evidence (the `App Paths\wordcraft.exe` registry entry the MSI writes). Portable copies
    /// leave no such entry, so this tells the two apart when both live under Program Files.
    fn windows_msi_installed(&self, _exe: &str) -> bool {
        false
    }
    /// Packagers and locked-down runs can turn the automatic check off entirely
    /// (`WORDCRAFT_NO_UPDATE_CHECK`): no release request is ever made.
    fn no_update_check(&self) -> bool {
        false
    }
}

/// Install-detection probes are bounded so a stuck package or registry helper cannot leave the
/// update worker busy and block the window from closing.
#[cfg(any(target_os = "linux", target_os = "windows", test))]
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Run a helper program with captured output and a hard deadline. Capturing is essential: a probe
/// that inherited our stdout (such as `dpkg-query -S`) would corrupt `wordcraft-cli update --check`'s
/// JSON response, which must stay one value.
#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn run_quiet(program: &str, args: &[&str], timeout: std::time::Duration) -> Option<std::process::Output> {
    use std::process::{Command, Stdio};
    let child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    wait_with_deadline(child, timeout)
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn wait_with_deadline(mut child: std::process::Child, timeout: std::time::Duration) -> Option<std::process::Output> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            // Exited: collect the captured pipes under the time left, not a fresh deadline, so the
            // whole probe stays within `timeout` even when the child runs close to the limit.
            Ok(Some(_)) => {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                return collect_output(child, remaining);
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

/// Read a helper's captured pipes after it exits, but no longer than `timeout`. A helper that
/// spawns a descendant inheriting our stdout/stderr can leave those pipes open after the helper
/// itself is gone, so `wait_with_output()` alone could hang the update worker forever despite the
/// child deadline (#15 review). The blocking read runs on its own thread so the deadline still
/// applies; if it overruns, the result is dropped and `None` returned.
#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn collect_output(child: std::process::Child, timeout: std::time::Duration) -> Option<std::process::Output> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("wordcraft-probe-read".into())
        .spawn(move || {
            let _ = tx.send(child.wait_with_output());
        })
        .ok()?;
    match rx.recv_timeout(timeout) {
        Ok(result) => result.ok(),
        Err(_) => None,
    }
}

#[cfg(target_os = "linux")]
fn linux_deb_package_query(exe: &str) -> bool {
    if exe.is_empty() || exe.len() > 512 {
        return false;
    }
    run_quiet("dpkg-query", &["-S", exe], QUERY_TIMEOUT).is_some_and(|out| out.status.success())
}

#[cfg(not(target_os = "linux"))]
fn linux_deb_package_query(_exe: &str) -> bool {
    false
}

/// Does the Windows registry say an MSI installed this exact executable? The MSI writes the
/// installed `wordcraft.exe`'s full path to `App Paths` (`packaging/windows/wordcraft.wxs`), which a
/// portable copy never does. An x86 package on 64-bit Windows lands in the 32-bit registry view, so
/// both views are queried.
#[cfg(target_os = "windows")]
fn windows_msi_path_matches(exe: &str) -> bool {
    if exe.is_empty() || exe.len() > 1024 {
        return false;
    }
    ["/reg:64", "/reg:32"].iter().any(|view| {
        let output =
            run_quiet("reg", &["query", r"HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths\wordcraft.exe", "/ve", view], QUERY_TIMEOUT);
        match output {
            Some(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.split("REG_SZ")
                    .nth(1)
                    .and_then(|rest| rest.lines().next())
                    .map(str::trim)
                    .is_some_and(|registered| same_windows_path(registered, exe))
            }
            _ => false,
        }
    })
}

#[cfg(not(target_os = "windows"))]
fn windows_msi_path_matches(_exe: &str) -> bool {
    false
}

/// Compare two Windows paths for identity, ignoring separators, quotes and case.
#[cfg(target_os = "windows")]
fn same_windows_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_matches('"').replace('/', "\\").to_ascii_lowercase();
    !a.is_empty() && norm(a) == norm(b)
}

/// Production environment reader.
pub struct SystemEnv;

impl EnvRead for SystemEnv {
    fn linux_deb_package(&self, exe: &str) -> bool {
        linux_deb_package_query(exe)
    }
    fn windows_msi_installed(&self, exe: &str) -> bool {
        windows_msi_path_matches(exe)
    }
    fn flatpak(&self) -> bool {
        std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
    }
    fn appimage(&self) -> Option<String> {
        std::env::var("APPIMAGE").ok().filter(|s| !s.is_empty())
    }
    fn install_override(&self) -> Option<InstallKind> {
        // Packagers may force Flatpak/AppImage/unknown; never grant MSI/.deb/.app without registry,
        // dpkg or `/Applications` evidence (tests inject [`EnvRead`] instead).
        match std::env::var("WORDCRAFT_INSTALL").ok().as_deref() {
            Some("flatpak") => Some(InstallKind::Flatpak),
            Some("appimage") => Some(InstallKind::AppImage),
            Some("unknown") | Some("portable") => Some(InstallKind::Unknown),
            _ => None,
        }
    }
    fn windows(&self) -> bool {
        cfg!(target_os = "windows")
    }
    fn macos(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn linux(&self) -> bool {
        cfg!(all(unix, not(target_os = "macos")))
    }
    fn no_update_check(&self) -> bool {
        // Build-time (a packager ships updates another way) or a single run's environment. An
        // empty value or `0` (as in the runtime variable) does not disable the check.
        if matches!(option_env!("WORDCRAFT_NO_UPDATE_CHECK"), Some(v) if !v.is_empty() && v != "0") {
            return true;
        }
        matches!(std::env::var("WORDCRAFT_NO_UPDATE_CHECK"), Ok(v) if !v.is_empty() && v != "0")
    }
}

static SYSTEM_ENV: SystemEnv = SystemEnv;

/// The environment reader for a session: its test/override hook, else the real system.
pub fn env_of(s: &crate::Session) -> &dyn EnvRead {
    match s.update_env.as_ref() {
        Some(env) => env.as_ref(),
        None => &SYSTEM_ENV,
    }
}

/// `v1.2.3`, `1.2` → version; pre-releases and junk → `None`.
pub fn parse_version(tag: &str) -> Option<Version> {
    if tag.len() > 64 {
        return None;
    }
    let tag = tag.trim();
    let tag = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let core = tag.split_once('+').map_or(tag, |(c, _)| c);
    let mut parts = core.split('.');
    let mut next = |required: bool| -> Option<u64> {
        match parts.next() {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok(),
            None if !required => Some(0),
            _ => None,
        }
    };
    let v = (next(true)?, next(true)?, next(false)?);
    parts.next().is_none().then_some(v)
}

pub fn version_text(v: Version) -> String {
    let (a, b, c) = v;
    format!("{a}.{b}.{c}")
}

/// A published release newer than the running build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub page_url: String,
}

/// Result of a check (returned by `file.checkForUpdates`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub status: CheckStatus,
    pub current: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseOffer>,
    /// A catalogued English message the interface translates by key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The technical cause, logged for support, never shown raw (may contain interpolated text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
    /// An applied install succeeded but the platform needs a restart to finish (MSI 3010/1641).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reboot_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Current,
    Available,
    Throttled,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseOffer {
    pub version: String,
    pub notes: String,
    pub page_url: String,
    pub install_kind: InstallKind,
    pub one_click: bool,
    /// Show **Update** (verified download); macOS opens the `.dmg` in Finder instead of replacing the bundle.
    pub verified_update: bool,
    pub external_updater: bool,
}

/// Whether a scheduled check may run now.
pub fn due(check_on_startup: bool, last_unix: u64, now_unix: u64, force: bool) -> bool {
    if force {
        return true;
    }
    if !check_on_startup {
        return false;
    }
    now_unix != 0 && now_unix.checked_sub(last_unix).is_none_or(|e| e >= INTERVAL_SECS)
}

/// Parse GitHub latest-release JSON into a stable release (no draft/pre-release).
pub fn parse_latest_release(json: &[u8], current: Version) -> Result<Option<ReleaseInfo>, String> {
    if json.len() > MAX_JSON {
        return Err("response too large".into());
    }
    let v: Value = serde_json::from_slice(json).map_err(|_| "invalid JSON".to_string())?;
    if v.get("draft").and_then(Value::as_bool) != Some(false) {
        return Ok(None);
    }
    if v.get("prerelease").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or_else(|| "missing tag".to_string())?;
    let version = parse_version(tag).ok_or_else(|| "unrecognized version".to_string())?;
    if version <= current {
        return Ok(None);
    }
    let notes = v.get("body").and_then(Value::as_str).unwrap_or("").chars().take(32_768).collect();
    let page_url = v
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|u| u.len() <= 256 && u.starts_with(RELEASES_PAGE) && u.bytes().all(|b| b.is_ascii_graphic()))
        .map_or_else(|| format!("{RELEASES_PAGE}latest"), str::to_string);
    Ok(Some(ReleaseInfo { version, tag: tag.to_string(), notes, page_url }))
}

/// Offer a release unless the user skipped this version.
pub fn offer(release: ReleaseInfo, _skipped: &str, kind: InstallKind) -> ReleaseOffer {
    let ver = version_text(release.version);
    ReleaseOffer {
        version: ver,
        notes: release.notes,
        page_url: release.page_url,
        install_kind: kind,
        one_click: kind.supports_one_click(),
        verified_update: kind.offers_verified_update(),
        external_updater: kind.external_updater(),
    }
}

pub fn skipped(release: &ReleaseInfo, skipped: &str) -> bool {
    parse_version(skipped) == Some(release.version)
}

/// Pick the release asset file name for this install kind and CPU arch.
pub fn asset_name(version: Version, kind: InstallKind, arch: &str) -> Option<String> {
    let v = version_text(version);
    match kind {
        InstallKind::WindowsMsi => Some(format!("wordcraft-{v}-windows-{arch}.msi")),
        InstallKind::WindowsPortableZip => Some(format!("wordcraft-{v}-windows-{arch}-portable.zip")),
        InstallKind::MacDmg | InstallKind::MacApp => Some(format!("wordcraft-{v}-macos-universal.dmg")),
        InstallKind::LinuxDeb => Some(format!("wordcraft-{v}-linux-{arch}.deb")),
        InstallKind::Flatpak | InstallKind::AppImage | InstallKind::Unknown => None,
    }
}

/// Map Rust's target arch to release file arch segment.
pub fn target_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "x86",
        "arm" => "arm64",
        _ => "x86_64",
    }
}

/// Windows release arch label (x64, x86, arm64).
pub fn windows_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => "x64",
    }
}

pub fn pick_asset_name(version: Version, kind: InstallKind) -> Option<String> {
    pick_asset_name_with_arch(version, kind, host_release_arch())
}

/// Release arch label for the running build (or tests may inject another through the session).
pub fn host_release_arch() -> &'static str {
    if cfg!(target_os = "windows") { windows_arch() } else { target_arch() }
}

pub fn pick_asset_name_with_arch(version: Version, kind: InstallKind, arch: &str) -> Option<String> {
    asset_name(version, kind, arch)
}

pub fn release_arch_for(session: &crate::Session) -> String {
    session.update_release_arch.as_deref().filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| host_release_arch().to_string())
}

/// Parse `SHA256SUMS.txt` and return the hash for `file_name`.
pub fn checksum_for(sums: &[u8], file_name: &str) -> Result<Option<String>, String> {
    if sums.len() > MAX_JSON {
        return Err("checksums too large".into());
    }
    let want = file_name.trim();
    let text = std::str::from_utf8(sums).map_err(|_| "checksums are not UTF-8".to_string())?;
    for line in text.lines() {
        let line = line.split_once('#').map_or(line, |(a, _)| a).trim();
        if line.is_empty() {
            continue;
        }
        let Some((hash, name)) = line.split_once(|c: char| c.is_whitespace()) else { continue };
        let name = name.trim();
        if name == want && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(Some(hash.to_ascii_lowercase()));
        }
    }
    Ok(None)
}

/// Verify downloaded bytes against a lowercase hex SHA-256.
pub fn verify_sha256(data: &[u8], expected: &str) -> bool {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let mut h = Sha256::new();
    h.update(data);
    let got = h.finalize();
    let mut hex = String::with_capacity(64);
    for b in got {
        hex.push(hex_char(b >> 4));
        hex.push(hex_char(b & 0xf));
    }
    hex == expected
}

fn hex_char(n: u8) -> char {
    match n {
        0..=9 => char::from_u32('0' as u32 + n as u32).unwrap_or('0'),
        10..=15 => char::from_u32('a' as u32 + (n - 10) as u32).unwrap_or('a'),
        _ => '0',
    }
}

/// Find a release asset download URL by file name in the latest-release JSON.
pub fn asset_download_url(latest_json: &[u8], file_name: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(latest_json).ok()?;
    let assets = v.get("assets")?.as_array()?;
    for a in assets {
        if a.get("name").and_then(Value::as_str) == Some(file_name) {
            return a.get("browser_download_url").and_then(Value::as_str).filter(|u| u.starts_with("https://")).map(str::to_string);
        }
    }
    None
}

/// Build the checksum file URL from a release tag download link pattern.
pub fn checksums_download_url(latest_json: &[u8]) -> Option<String> {
    asset_download_url(latest_json, CHECKSUMS_NAME)
}

/// What the platform installer receives after a verified download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallHandoff {
    pub kind: InstallKind,
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub exe_path: String,
}

/// What an installer handoff produced. A successful install can still ask for a restart (Windows
/// MSI returns 3010/1641 when a reboot finishes replacing in-use files); that is reported so the
/// interface can tell the user rather than treating it as a failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    /// Installed, but the platform needs a restart to finish.
    pub reboot_required: bool,
}

/// HTTP GET through the session hook (`max_bytes` is enforced while reading).
pub fn http_get(session: &crate::Session, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let fetch = session.update_http.as_ref().ok_or_else(|| "update check is not available".to_string())?;
    let cap = max_bytes.min(MAX_DOWNLOAD);
    let data = fetch(url, cap)?;
    if data.len() > cap {
        return Err("download too large".into());
    }
    Ok(data)
}

pub type HttpGet = Arc<dyn Fn(&str, usize) -> Result<Vec<u8>, String> + Send + Sync>;
pub type InstallFn = Arc<dyn Fn(InstallHandoff) -> Result<InstallOutcome, String> + Send + Sync>;
pub type EnvReadBox = Arc<dyn EnvRead + Send + Sync>;

/// Unix seconds for throttling (0 when unknown).
pub fn unix_now() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeEnv {
        flatpak: bool,
        appimage: Option<String>,
        override_kind: Option<InstallKind>,
        linux_deb: bool,
        os: &'static str,
        /// What the fake registry reports for the `App Paths` entry.
        windows_msi: bool,
    }

    impl EnvRead for FakeEnv {
        fn flatpak(&self) -> bool {
            self.flatpak
        }
        fn appimage(&self) -> Option<String> {
            self.appimage.clone()
        }
        fn install_override(&self) -> Option<InstallKind> {
            self.override_kind
        }
        fn windows(&self) -> bool {
            self.os == "windows"
        }
        fn macos(&self) -> bool {
            self.os == "macos"
        }
        fn linux(&self) -> bool {
            self.os == "linux"
        }
        fn linux_deb_package(&self, _exe: &str) -> bool {
            self.linux_deb
        }
        fn windows_msi_installed(&self, _exe: &str) -> bool {
            self.windows_msi
        }
    }

    fn win_env(windows_msi: bool) -> FakeEnv {
        FakeEnv { flatpak: false, appimage: None, override_kind: None, linux_deb: false, os: "windows", windows_msi }
    }

    #[test]
    fn install_kind_detection() {
        let e = FakeEnv { flatpak: true, appimage: None, override_kind: None, linux_deb: false, os: "linux", windows_msi: false };
        assert_eq!(detect_install_kind("/usr/bin/wordcraft", &e), InstallKind::Flatpak);
        let e =
            FakeEnv { flatpak: false, appimage: Some("/x.AppImage".into()), override_kind: None, linux_deb: false, os: "linux", windows_msi: false };
        assert_eq!(detect_install_kind("/tmp/wordcraft", &e), InstallKind::AppImage);
        let e = FakeEnv { flatpak: false, appimage: None, override_kind: None, linux_deb: true, os: "linux", windows_msi: false };
        assert_eq!(detect_install_kind("/usr/bin/wordcraft", &e), InstallKind::LinuxDeb);
        let e = FakeEnv { flatpak: false, appimage: None, override_kind: None, linux_deb: false, os: "linux", windows_msi: false };
        assert_eq!(detect_install_kind("/usr/bin/wordcraft", &e), InstallKind::Unknown);
        let e = FakeEnv { flatpak: false, appimage: None, override_kind: None, linux_deb: false, os: "macos", windows_msi: false };
        assert_eq!(detect_install_kind("/Applications/WordCraft.app/Contents/MacOS/wordcraft", &e), InstallKind::MacApp);
        assert_eq!(detect_install_kind("/Users/me/WordCraft.app/Contents/MacOS/wordcraft", &e), InstallKind::Unknown);
        assert_eq!(
            detect_install_kind("/Users/me/Applications/WordCraft.app/Contents/MacOS/wordcraft", &e),
            InstallKind::Unknown,
            "~/Applications must not qualify for one-click install"
        );
        let e =
            FakeEnv { flatpak: false, appimage: None, override_kind: Some(InstallKind::Unknown), linux_deb: false, os: "linux", windows_msi: false };
        assert_eq!(detect_install_kind("/build/wordcraft", &e), InstallKind::Unknown);
        let e =
            FakeEnv { flatpak: false, appimage: None, override_kind: Some(InstallKind::LinuxDeb), linux_deb: false, os: "linux", windows_msi: false };
        assert_eq!(
            detect_install_kind("/usr/bin/wordcraft", &e),
            InstallKind::LinuxDeb,
            "tests may inject install kind; production env override does not grant .deb without dpkg"
        );
    }

    /// MSI detection uses registry evidence, not path shape alone; portable paths fall back to the release page.
    #[test]
    fn windows_install_detection_uses_evidence_not_just_the_folder() {
        let e = win_env(true);
        assert_eq!(detect_install_kind(r"C:\Program Files\WordCraft\wordcraft.exe", &e), InstallKind::WindowsMsi);
        let no_evidence = win_env(false);
        // A folder name containing "portable" is not evidence of a dedicated portable install.
        assert_eq!(detect_install_kind(r"C:\Program Files\wordcraft-0.6.0-windows-x64-portable\wordcraft.exe", &no_evidence), InstallKind::Unknown);
        assert_eq!(detect_install_kind(r"C:\Program Files\WordCraft\wordcraft.exe", &no_evidence), InstallKind::Unknown);
        assert_eq!(
            detect_install_kind(r"C:\Users\portable_user\Downloads\wordcraft.exe", &e),
            InstallKind::WindowsMsi,
            "registry evidence wins over path shape (no dedicated portable-zip install type)"
        );
        assert_eq!(detect_install_kind(r"C:\Users\portable_user\Downloads\wordcraft.exe", &no_evidence), InstallKind::Unknown);
        assert_eq!(
            detect_install_kind(r"D:\wordcraft-0.6.0-windows-x64-portable\wordcraft.exe", &e),
            InstallKind::WindowsMsi,
            "a portable-looking folder name is not install-type evidence"
        );
        assert_eq!(detect_install_kind(r"D:\wordcraft-0.6.0-windows-x64-portable\wordcraft.exe", &no_evidence), InstallKind::Unknown);
        assert_eq!(
            detect_install_kind(r"D:\Apps\WordCraft\wordcraft.exe", &e),
            InstallKind::WindowsMsi,
            "registry evidence applies outside Program Files"
        );
        assert_eq!(detect_install_kind(r"D:\Apps\WordCraft\wordcraft.exe", &no_evidence), InstallKind::Unknown);
    }

    #[test]
    fn asset_names_per_kind() {
        let v = (0, 6, 1);
        assert_eq!(asset_name(v, InstallKind::LinuxDeb, "x86_64").as_deref(), Some("wordcraft-0.6.1-linux-x86_64.deb"));
        assert_eq!(asset_name(v, InstallKind::LinuxDeb, "aarch64").as_deref(), Some("wordcraft-0.6.1-linux-aarch64.deb"));
        assert_eq!(asset_name(v, InstallKind::WindowsMsi, "x64").as_deref(), Some("wordcraft-0.6.1-windows-x64.msi"));
        assert!(asset_name(v, InstallKind::Unknown, "x86_64").is_none());
        assert_eq!(pick_asset_name_with_arch(v, InstallKind::LinuxDeb, "aarch64").as_deref(), Some("wordcraft-0.6.1-linux-aarch64.deb"));
        assert_eq!(pick_asset_name(v, InstallKind::LinuxDeb).as_deref(), asset_name(v, InstallKind::LinuxDeb, host_release_arch()).as_deref());
    }

    #[test]
    fn versions_and_offers() {
        let json = br#"{"tag_name":"v0.7.0","draft":false,"prerelease":false,"body":"Notes","html_url":"https://github.com/storytold/wordcraft/releases/tag/v0.7.0","assets":[]}"#;
        let cur = (0, 6, 1);
        let r = parse_latest_release(json, cur).unwrap().unwrap();
        assert_eq!(r.version, (0, 7, 0));
        assert!(!skipped(&r, "0.6.1"));
        assert!(skipped(&r, "0.7.0"));
        assert_eq!(parse_latest_release(json, (0, 7, 0)).unwrap(), None);
    }

    #[test]
    fn throttle_and_setting_off() {
        let now = 1_800_000_000;
        assert!(!due(false, 0, now, false));
        assert!(!due(true, now - 100, now, false));
        assert!(due(true, now - 100, now, true));
        assert!(due(true, 0, now, false));
        assert!(!due(true, now - 60, now, false));
    }

    #[test]
    fn checksum_verify_and_refuse_mismatch() {
        let data = b"hello";
        let sums = b"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  wordcraft-0.6.1-linux-x86_64.deb\n";
        let hash = checksum_for(sums, "wordcraft-0.6.1-linux-x86_64.deb").unwrap().unwrap();
        assert!(verify_sha256(data, &hash));
        assert!(!verify_sha256(b"bye", &hash));
        assert!(checksum_for(sums, "missing.deb").unwrap().is_none());
    }

    #[test]
    fn unknown_kind_has_no_asset() {
        let v = (1, 0, 0);
        assert!(pick_asset_name(v, InstallKind::Unknown).is_none());
    }

    #[test]
    fn each_install_kind_picks_an_asset_name() {
        let v = (0, 7, 0);
        assert_eq!(asset_name(v, InstallKind::WindowsMsi, "x64").as_deref(), Some("wordcraft-0.7.0-windows-x64.msi"));
        assert_eq!(asset_name(v, InstallKind::WindowsPortableZip, "arm64").as_deref(), Some("wordcraft-0.7.0-windows-arm64-portable.zip"));
        assert_eq!(asset_name(v, InstallKind::MacDmg, "x86_64").as_deref(), Some("wordcraft-0.7.0-macos-universal.dmg"));
        assert_eq!(asset_name(v, InstallKind::MacApp, "aarch64").as_deref(), Some("wordcraft-0.7.0-macos-universal.dmg"));
    }

    /// The regression for the CLI-JSON contamination: a probe's output must land in the returned
    /// `Output`, never on our stdout (#15). `/bin/echo`/`cmd` are used because every host has one.
    #[cfg(any(unix, windows))]
    #[test]
    fn helper_output_is_captured_not_inherited() {
        #[cfg(unix)]
        let (program, args): (&str, Vec<&str>) = ("/bin/echo", vec!["wordcraft-capture-probe"]);
        #[cfg(windows)]
        let (program, args): (&str, Vec<&str>) = ("cmd", vec!["/C", "echo", "wordcraft-capture-probe"]);
        let out = run_quiet(program, &args, QUERY_TIMEOUT).expect("the helper runs");
        assert!(out.status.success(), "the helper succeeded");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("wordcraft-capture-probe"),
            "the helper's output is captured in the returned Output, not inherited"
        );
    }

    /// A stuck probe is killed at the deadline instead of hanging the update worker (#15).
    #[cfg(unix)]
    #[test]
    fn helper_is_killed_at_the_deadline() {
        let start = std::time::Instant::now();
        let out = run_quiet("/bin/sh", &["-c", "sleep 30"], std::time::Duration::from_millis(300));
        assert!(out.is_none(), "a stuck probe is killed, not waited on forever");
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "the deadline is enforced");
    }

    /// The child exits at once, but a descendant it spawned inherits the captured stdout/stderr
    /// and keeps those pipes open. Collecting the output must still stop at the deadline rather
    /// than hang on the pipes the descendant holds (#15 review).
    #[cfg(unix)]
    #[test]
    fn exited_helper_with_a_pipe_holding_descendant_is_bounded() {
        let start = std::time::Instant::now();
        let out = run_quiet("/bin/sh", &["-c", "sleep 30 & exit 0"], std::time::Duration::from_millis(300));
        assert!(out.is_none(), "pipe collection is bounded even when a descendant holds the pipes");
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "the deadline covers output collection");
    }
}
