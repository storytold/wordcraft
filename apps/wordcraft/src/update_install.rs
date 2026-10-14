//! One-click update handoff after a verified download (#15).
//!
//! Windows MSI and Linux `.deb` run the platform installer. macOS writes the verified `.dmg` to a
//! private temp file and opens it in Finder (`open`); the user drags WordCraft into Applications.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use wordcraft_engine::update::{InstallFn, InstallHandoff, InstallKind, InstallOutcome};

/// Staging directories all share this prefix, so a later sweep can find them.
const STAGING_PREFIX: &str = "wordcraft-update-";
/// How long a download kept for the manual macOS install is left in place before a later sweep
/// removes it. `open` returning does not mean Finder has finished mounting the disk image, so the
/// verified file is kept; a leftover from a previous update is cleared once it is certainly idle.
const STAGING_RETENTION: std::time::Duration = std::time::Duration::from_secs(3 * 24 * 60 * 60);

pub fn handoff() -> InstallFn {
    std::sync::Arc::new(run)
}

fn run(job: InstallHandoff) -> Result<InstallOutcome, String> {
    run_with(job, &install_into)
}

/// A platform handoff step: write the download into `staging` (done by [`install_into`]) and hand
/// it off. Split out so tests can drive the staging lifetime with a fake handoff.
type InstallAction<'a> = dyn Fn(&InstallHandoff, &Path) -> Result<InstallOutcome, String> + 'a;

fn run_with(job: InstallHandoff, action: &InstallAction<'_>) -> Result<InstallOutcome, String> {
    // Clear a leftover from an earlier manual install before creating this run's staging.
    sweep_stale_staging();
    let staging = private_staging_dir()?;
    let result = action(&job, &staging);
    finish_staging(job.kind, &staging, &result);
    result
}

/// Remove this run's staging unless a successful handoff still needs the download. A macOS `.dmg`
/// opened in Finder may be read after `open` returns, so it is kept for the manual install; every
/// other handoff (installer ran to completion, or failed) is done with the file.
fn finish_staging(kind: InstallKind, staging: &Path, result: &Result<InstallOutcome, String>) {
    if matches!(kind, InstallKind::MacDmg | InstallKind::MacApp) && result.is_ok() {
        return;
    }
    let _ = std::fs::remove_dir_all(staging);
}

/// Remove staging directories from earlier manual installs once they are old enough that Finder is
/// certainly done with the disk image. Called before each handoff and at start-up.
pub fn sweep_stale_staging() {
    sweep_stale_staging_in(&std::env::temp_dir(), STAGING_RETENTION, std::time::SystemTime::now());
}

fn sweep_stale_staging_in(parent: &Path, retention: std::time::Duration, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        if !is_staging_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() {
            continue;
        }
        let stale = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= retention);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether a directory name is one this program generated: [`STAGING_PREFIX`] followed by exactly
/// the 32 lowercase hex characters of [`random_hex`]`(16)`. Restricting the sweep to that exact
/// format keeps an unrelated directory that merely shares the prefix from being removed.
fn is_staging_name(name: &str) -> bool {
    let Some(hex) = name.strip_prefix(STAGING_PREFIX) else { return false };
    hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn install_into(job: &InstallHandoff, staging: &Path) -> Result<InstallOutcome, String> {
    let file_name = safe_name(&job.file_name);
    let path = write_exclusive_file(staging, &file_name, &job.bytes)?;
    match job.kind {
        InstallKind::WindowsMsi => windows_msi(&path),
        InstallKind::LinuxDeb => linux_deb(&path),
        InstallKind::MacDmg | InstallKind::MacApp => mac_open_dmg(&path),
        InstallKind::WindowsPortableZip | InstallKind::Flatpak | InstallKind::AppImage | InstallKind::Unknown => {
            Err("This install type must use the release page.".into())
        }
    }
}

/// A private directory only this process can create (not a predictable shared path).
pub fn private_staging_dir() -> Result<PathBuf, String> {
    let hex = random_hex(16)?;
    let path = std::env::temp_dir().join(format!("{STAGING_PREFIX}{hex}"));
    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    let mut hex = String::with_capacity(bytes * 2);
    for b in buf {
        hex.push(hex_digit(b >> 4));
        hex.push(hex_digit(b & 0xf));
    }
    Ok(hex)
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => char::from_u32('0' as u32 + n as u32).unwrap_or('0'),
        10..=15 => char::from_u32('a' as u32 + (n - 10) as u32).unwrap_or('a'),
        _ => '0',
    }
}

/// Write bytes to a new file inside `dir`; refuses if the name already exists.
pub fn write_exclusive_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err("invalid file name".into());
    }
    let path = dir.join(name);
    let mut f = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn safe_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_').collect()
}

fn wait_child(mut child: std::process::Child, secs: u64) -> Result<std::process::ExitStatus, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the installer took too long".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
const INSTALLER_WAIT_SECS: u64 = 60 * 60;

#[cfg(any(target_os = "windows", test))]
pub const MSI_SUCCESS: u32 = 0;
#[cfg(any(target_os = "windows", test))]
pub const MSI_SUCCESS_REBOOT_REQUIRED: u32 = 3010;
#[cfg(any(target_os = "windows", test))]
pub const MSI_SUCCESS_REBOOT_INITIATED: u32 = 1641;
#[cfg(any(target_os = "windows", test))]
pub const MSI_CANCELLED: u32 = 1602;

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsiOutcome {
    Installed,
    InstalledRebootRequired,
    Cancelled,
    Failed(u32),
}

#[cfg(any(target_os = "windows", test))]
pub fn msi_outcome(code: u32) -> MsiOutcome {
    match code {
        MSI_SUCCESS => MsiOutcome::Installed,
        MSI_SUCCESS_REBOOT_REQUIRED | MSI_SUCCESS_REBOOT_INITIATED => MsiOutcome::InstalledRebootRequired,
        MSI_CANCELLED => MsiOutcome::Cancelled,
        other => MsiOutcome::Failed(other),
    }
}

#[cfg(any(target_os = "windows", test))]
pub fn msi_args(path: &str) -> Vec<String> {
    vec!["/i".into(), path.into(), "/passive".into(), "/norestart".into(), "MSIRESTARTMANAGERCONTROL=DisableShutdown".into()]
}

#[cfg(any(target_os = "windows", test))]
pub trait MsiRunner {
    fn run(&self, args: &[String], timeout_secs: u64) -> Result<u32, String>;
}

#[cfg(any(target_os = "windows", test))]
const MSI_WAIT_SECS: u64 = 60 * 60;

#[cfg(any(target_os = "windows", test))]
fn windows_msi_with(runner: &dyn MsiRunner, path: &Path) -> Result<InstallOutcome, String> {
    let p = path.to_str().ok_or("bad path")?;
    let code = runner.run(&msi_args(p), MSI_WAIT_SECS)?;
    match msi_outcome(code) {
        MsiOutcome::Installed => Ok(InstallOutcome::default()),
        MsiOutcome::InstalledRebootRequired => {
            log::info!("WordCraft was updated; restart the computer to finish (MSI code {code})");
            Ok(InstallOutcome { reboot_required: true })
        }
        MsiOutcome::Cancelled => Err("The update was cancelled.".into()),
        MsiOutcome::Failed(code) => Err(format!("msiexec exited with code {code}")),
    }
}

#[cfg(target_os = "windows")]
struct RealMsiRunner;

#[cfg(target_os = "windows")]
impl MsiRunner for RealMsiRunner {
    fn run(&self, args: &[String], timeout_secs: u64) -> Result<u32, String> {
        let child = Command::new("msiexec").args(args).spawn().map_err(|e| e.to_string())?;
        let status = wait_child(child, timeout_secs)?;
        status.code().map(|c| c as u32).ok_or_else(|| "msiexec was terminated by a signal".to_string())
    }
}

#[cfg(target_os = "windows")]
fn windows_msi(path: &Path) -> Result<InstallOutcome, String> {
    windows_msi_with(&RealMsiRunner, path)
}

#[cfg(not(target_os = "windows"))]
fn windows_msi(_path: &Path) -> Result<InstallOutcome, String> {
    Err("MSI install is only on Windows".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_deb(path: &Path) -> Result<InstallOutcome, String> {
    let p = path.to_str().ok_or("bad path")?;
    let child = Command::new("pkexec").args(["dpkg", "-i", p]).spawn().map_err(|e| e.to_string())?;
    let status = wait_child(child, INSTALLER_WAIT_SECS)?;
    if status.success() { Ok(InstallOutcome::default()) } else { Err("dpkg install was cancelled or failed.".into()) }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn linux_deb(_path: &Path) -> Result<InstallOutcome, String> {
    Err(".deb install is only on Linux".into())
}

/// Opens a verified `.dmg` in Finder (macOS only).
#[cfg(any(target_os = "macos", test))]
pub trait DmgOpener {
    fn open_in_finder(&self, dmg: &Path) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
const OPEN_DMG_WAIT_SECS: u64 = 60;

#[cfg(target_os = "macos")]
struct RealDmgOpener;

#[cfg(target_os = "macos")]
impl DmgOpener for RealDmgOpener {
    fn open_in_finder(&self, dmg: &Path) -> Result<(), String> {
        let p = dmg.to_str().ok_or("bad path")?;
        let child = Command::new("open").arg(p).spawn().map_err(|e| e.to_string())?;
        let status = wait_child(child, OPEN_DMG_WAIT_SECS)?;
        if status.success() { Ok(()) } else { Err("Could not open the disk image in Finder.".into()) }
    }
}

#[cfg(any(target_os = "macos", test))]
fn mac_open_dmg_with(path: &Path, opener: &dyn DmgOpener) -> Result<InstallOutcome, String> {
    opener.open_in_finder(path)?;
    Ok(InstallOutcome::default())
}

#[cfg(target_os = "macos")]
fn mac_open_dmg(path: &Path) -> Result<InstallOutcome, String> {
    mac_open_dmg_with(path, &RealDmgOpener)
}

#[cfg(not(target_os = "macos"))]
fn mac_open_dmg(_path: &Path) -> Result<InstallOutcome, String> {
    Err("disk image install is only on macOS".into())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn exclusive_file_refuses_existing() {
        let dir = private_staging_dir().unwrap();
        write_exclusive_file(&dir, "pkg.deb", b"one").unwrap();
        assert!(write_exclusive_file(&dir, "pkg.deb", b"two").is_err());
    }

    #[test]
    fn staging_dirs_are_unique() {
        let a = private_staging_dir().unwrap();
        let b = private_staging_dir().unwrap();
        assert_ne!(a, b);
    }

    #[cfg(unix)]
    #[test]
    fn staging_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = private_staging_dir().unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "staging must be private");
    }

    struct FakeDmgOpener {
        opened: RefCell<Option<PathBuf>>,
        err: Option<String>,
    }

    impl DmgOpener for FakeDmgOpener {
        fn open_in_finder(&self, dmg: &Path) -> Result<(), String> {
            if let Some(e) = &self.err {
                return Err(e.clone());
            }
            *self.opened.borrow_mut() = Some(dmg.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn mac_open_dmg_uses_opener() {
        let dir = private_staging_dir().unwrap();
        let dmg = write_exclusive_file(&dir, "wordcraft.dmg", b"x").unwrap();
        let fake = FakeDmgOpener { opened: RefCell::new(None), err: None };
        mac_open_dmg_with(&dmg, &fake).unwrap();
        assert_eq!(fake.opened.borrow().as_ref(), Some(&dmg));
    }

    #[test]
    fn mac_open_dmg_propagates_failure() {
        let dir = private_staging_dir().unwrap();
        let dmg = write_exclusive_file(&dir, "wordcraft.dmg", b"x").unwrap();
        let fake = FakeDmgOpener { opened: RefCell::new(None), err: Some("open failed".into()) };
        assert!(mac_open_dmg_with(&dmg, &fake).is_err());
    }

    fn job(kind: InstallKind, name: &str) -> InstallHandoff {
        InstallHandoff { kind, file_name: name.into(), bytes: b"x".to_vec(), exe_path: String::new() }
    }

    /// The verified macOS download must outlive `run`: Finder may still be mounting the disk image
    /// when `open` returns, so removing it immediately races. Other handoffs consume the file.
    #[test]
    fn macos_handoff_keeps_the_verified_download_while_others_remove_it() {
        let seen = RefCell::new(None);
        let record = |staging: &Path| {
            *seen.borrow_mut() = Some(staging.to_path_buf());
        };

        // A successful macOS handoff keeps the staging directory (and the `.dmg` in it).
        let mac_ok = |j: &InstallHandoff, staging: &Path| {
            *seen.borrow_mut() = Some(staging.to_path_buf());
            write_exclusive_file(staging, &j.file_name, &j.bytes)?;
            Ok(InstallOutcome::default())
        };
        run_with(job(InstallKind::MacDmg, "wordcraft.dmg"), &mac_ok).unwrap();
        let staging = seen.borrow().clone().expect("staging seen");
        assert!(staging.join("wordcraft.dmg").exists(), "the verified dmg is kept for Finder");
        assert!(staging.exists(), "the staging directory survives the handoff");
        let _ = std::fs::remove_dir_all(&staging);

        // A failed macOS handoff leaves nothing behind.
        let mac_err = |_j: &InstallHandoff, staging: &Path| {
            record(staging);
            Err::<InstallOutcome, String>("open failed".into())
        };
        run_with(job(InstallKind::MacDmg, "wordcraft.dmg"), &mac_err).unwrap_err();
        let staging = seen.borrow().clone().unwrap();
        assert!(!staging.exists(), "a failed handoff removes its staging");
        let _ = std::fs::remove_dir_all(&staging);

        // A `.deb` handoff ran the installer to completion; the staging is removed.
        let deb_ok = |j: &InstallHandoff, staging: &Path| {
            record(staging);
            write_exclusive_file(staging, &j.file_name, &j.bytes)?;
            Ok(InstallOutcome::default())
        };
        run_with(job(InstallKind::LinuxDeb, "wordcraft.deb"), &deb_ok).unwrap();
        let staging = seen.borrow().clone().unwrap();
        assert!(!staging.exists(), "a consumed installer download is removed");
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn stale_staging_is_swept_but_fresh_and_foreign_entries_are_kept() {
        let parent = std::env::temp_dir().join(format!("wordcraft-sweep-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        let staging = |tag: char| parent.join(format!("{STAGING_PREFIX}{}", tag.to_string().repeat(32)));
        let fresh = staging('a');
        let old = staging('b');
        let foreign = parent.join("something-else");
        // Shares the prefix but is not the generated format, so it is none of our business.
        let prefix_only = parent.join(format!("{STAGING_PREFIX}not-a-staging"));
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::create_dir_all(&prefix_only).unwrap();

        // Nothing is old enough yet, and a non-staging directory is never touched.
        sweep_stale_staging_in(&parent, std::time::Duration::from_secs(3600), std::time::SystemTime::now());
        assert!(fresh.exists(), "a fresh download is left for Finder");
        assert!(foreign.exists(), "only update staging directories are swept");
        assert!(prefix_only.exists(), "a directory sharing only the prefix is never swept");

        // Past the retention window the staging directories are stale and removed.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(7200);
        sweep_stale_staging_in(&parent, std::time::Duration::from_secs(3600), later);
        assert!(!old.exists(), "a stale download is removed");
        assert!(!fresh.exists(), "a download past the retention is removed");
        assert!(foreign.exists(), "a foreign directory is never touched");
        assert!(prefix_only.exists(), "a prefix-only directory is never touched even when old");
        let _ = std::fs::remove_dir_all(&parent);
    }

    struct FakeMsi {
        code: u32,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl MsiRunner for FakeMsi {
        fn run(&self, args: &[String], _timeout_secs: u64) -> Result<u32, String> {
            self.calls.borrow_mut().push(args.to_vec());
            Ok(self.code)
        }
    }

    fn run_msi(code: u32) -> (Result<InstallOutcome, String>, Vec<String>) {
        let fake = FakeMsi { code, calls: RefCell::new(Vec::new()) };
        let outcome = windows_msi_with(&fake, Path::new("C:\\tmp\\wordcraft.msi"));
        let args = fake.calls.into_inner().into_iter().next().unwrap_or_default();
        (outcome, args)
    }

    #[test]
    fn msi_args_suppress_restart_and_forced_shutdown() {
        let args = msi_args("pkg.msi");
        assert_eq!(args.first().map(String::as_str), Some("/i"));
        assert_eq!(args.get(1).map(String::as_str), Some("pkg.msi"));
        assert!(args.iter().any(|a| a == "/passive"), "{args:?}");
        assert!(args.iter().any(|a| a == "/norestart"), "an installer restart must be suppressed: {args:?}");
        assert!(
            args.iter().any(|a| a.eq_ignore_ascii_case("MSIRESTARTMANAGERCONTROL=DisableShutdown")),
            "the installer must not close WordCraft to replace files: {args:?}"
        );
    }

    #[test]
    fn msi_zero_is_an_install() {
        let (outcome, args) = run_msi(MSI_SUCCESS);
        assert_eq!(outcome.unwrap(), InstallOutcome::default());
        assert!(args.iter().any(|a| a == "/norestart"), "the runner receives the arguments: {args:?}");
        assert_eq!(msi_outcome(MSI_SUCCESS), MsiOutcome::Installed);
    }

    #[test]
    fn msi_3010_is_success_with_a_deferred_restart() {
        let (outcome, _) = run_msi(MSI_SUCCESS_REBOOT_REQUIRED);
        assert!(outcome.expect("3010 is success, not failure").reboot_required);
        assert_eq!(msi_outcome(MSI_SUCCESS_REBOOT_REQUIRED), MsiOutcome::InstalledRebootRequired);
        assert_eq!(msi_outcome(MSI_SUCCESS_REBOOT_INITIATED), MsiOutcome::InstalledRebootRequired);
    }

    #[test]
    fn msi_cancelled_is_an_error() {
        let (outcome, _) = run_msi(MSI_CANCELLED);
        let err = outcome.unwrap_err();
        assert!(err.to_ascii_lowercase().contains("cancel"), "{err}");
        assert_eq!(msi_outcome(MSI_CANCELLED), MsiOutcome::Cancelled);
    }

    #[test]
    fn msi_failure_is_an_error() {
        let (outcome, _) = run_msi(1603);
        let err = outcome.unwrap_err();
        assert!(err.contains("1603"), "{err}");
        assert_eq!(msi_outcome(1603), MsiOutcome::Failed(1603));
    }

    #[test]
    fn msi_runner_start_failure_is_propagated() {
        struct Failing;
        impl MsiRunner for Failing {
            fn run(&self, _args: &[String], _timeout_secs: u64) -> Result<u32, String> {
                Err("msiexec not found".into())
            }
        }
        assert!(windows_msi_with(&Failing, Path::new("x.msi")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn deadline_terminates_the_child_promptly() {
        let child = Command::new("/bin/sh").args(["-c", "sleep 30"]).spawn().expect("spawn sleep");
        let start = std::time::Instant::now();
        assert!(wait_child(child, 1).is_err(), "the deadline is enforced");
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "termination does not wait for the sleep");
    }
}
