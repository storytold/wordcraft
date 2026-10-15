//! `file.checkForUpdates` — desktop update check and apply (#15).

use serde_json::{Value, json};

use crate::update::{
    CheckResult, CheckStatus, InstallHandoff, InstallOutcome, LATEST_API, MAX_DOWNLOAD, MAX_JSON, MSG_APPLY_FAILED, MSG_BAD_RELEASE,
    MSG_CHECK_FAILED, MSG_CHECKSUM_MISMATCH, MSG_CHECKSUM_MISSING, MSG_CHECKSUMS_UNREADABLE, MSG_NO_CHECKSUMS, MSG_NO_INSTALL_FN, MSG_NO_INSTALLER,
    ReleaseInfo, UpdateError, checksum_for, checksums_download_url, detect_install_kind, due, env_of, http_get, offer, parse_latest_release,
    parse_version, pick_asset_name_with_arch, release_arch_for, skipped, verify_sha256, version_text,
};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("file.checkForUpdates", "Check for Updates", "Help", check).params(
            r#"{"force"?: bool (ignore the once-a-day throttle), "apply"?: bool (download, verify checksum and hand off to the installer), "startup"?: bool (respect check-on-startup and throttle; default false for explicit checks)}"#,
        )
        .pure(),
        CommandSpec::new("file.updatePrefs", "Update Preferences", "Help", prefs).params(
            r#"{"checkOnStartup"?: bool, "askedStartup"?: bool, "skipVersion"?: string (skip notices for this version)}"#,
        )
        .pure(),
    ]
}

fn prefs(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(on) = p::bool(v, "checkOnStartup") {
        s.update_prefs.check_on_startup = on;
    }
    if let Some(asked) = p::bool(v, "askedStartup") {
        s.update_prefs.asked_startup = asked;
    }
    if let Some(ver) = p::str(v, "skipVersion") {
        let ver = ver.chars().take(64).collect::<String>();
        s.update_prefs.skipped_version = ver;
    }
    Ok(json!({
        "checkOnStartup": s.update_prefs.check_on_startup,
        "askedStartup": s.update_prefs.asked_startup,
        "skippedVersion": s.update_prefs.skipped_version,
    }))
}

fn check(s: &mut Session, v: &Value) -> CmdResult {
    let force = p::bool(v, "force").unwrap_or(false);
    let apply = p::bool(v, "apply").unwrap_or(false);
    let startup = p::bool(v, "startup").unwrap_or(false);
    let current = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
    let current_text = version_text(current);
    let now = (s.update_now_unix)();
    let check_on_startup = s.update_prefs.check_on_startup;
    let last_check = s.update_prefs.last_check_unix;
    let skipped_ver = s.update_prefs.skipped_version.clone();

    // Asking to install with unsaved work must never reach the network: the interface prompts to
    // save first, and this is the backstop for programmatic callers.
    if apply && s.dirty {
        return Err(CmdError::Failed("Save or discard unsaved changes before updating.".into()));
    }

    if startup && env_of(s).no_update_check() {
        return Ok(json_result(throttled(&current_text)));
    }
    if startup && !due(check_on_startup, last_check, now, false) {
        return Ok(json_result(throttled(&current_text)));
    }
    if !force && !startup && last_check != 0 {
        let elapsed = now.checked_sub(last_check);
        if now != 0 && elapsed.is_some_and(|e| e < crate::update::INTERVAL_SECS) {
            return Ok(json_result(throttled(&current_text)));
        }
    }

    if startup && !check_on_startup {
        return Ok(json_result(throttled(&current_text)));
    }

    s.update_prefs.last_check_unix = now;

    let json = match http_get(s, LATEST_API, MAX_JSON) {
        Ok(b) => b,
        Err(e) => return Ok(json_result(failed(&current_text, MSG_CHECK_FAILED, e))),
    };

    let release = match parse_latest_release(&json, current) {
        Ok(Some(r)) => r,
        Ok(None) => {
            return Ok(json_result(CheckResult {
                status: CheckStatus::Current,
                current: current_text,
                release: None,
                error: None,
                error_detail: None,
                reboot_required: false,
            }));
        }
        Err(e) => return Ok(json_result(failed(&current_text, MSG_BAD_RELEASE, e))),
    };

    if skipped(&release, &skipped_ver) {
        return Ok(json_result(CheckResult {
            status: CheckStatus::Current,
            current: current_text,
            release: None,
            error: None,
            error_detail: None,
            reboot_required: false,
        }));
    }

    let exe = s.update_exe.as_deref().unwrap_or("");
    let kind = install_kind(s, exe);
    let offer = offer(release.clone(), &skipped_ver, kind);

    let mut reboot_required = false;
    if apply {
        if kind.external_updater() || kind == crate::update::InstallKind::Unknown {
            return Err(CmdError::Failed("Open the release page in your browser to update.".into()));
        }
        match download_and_install(s, &json, &release, kind) {
            Ok(outcome) => reboot_required = outcome.reboot_required,
            Err(e) => {
                return Ok(json_result(CheckResult {
                    status: CheckStatus::Error,
                    current: current_text,
                    release: Some(offer),
                    error: Some(e.message.into()),
                    error_detail: Some(e.detail),
                    reboot_required: false,
                }));
            }
        }
        if kind.supports_one_click() {
            s.ui_requests.push(json!({"quit": true}));
            if reboot_required {
                s.ui_requests.push(json!({"updateReboot": true}));
            }
        } else if matches!(kind, crate::update::InstallKind::MacApp | crate::update::InstallKind::MacDmg) {
            s.ui_requests.push(json!({"updateMacDmg": true}));
        }
    } else {
        s.ui_requests.push(json!({
            "update": {
                "version": offer.version,
                "notes": offer.notes,
                "pageUrl": offer.page_url,
                "oneClick": offer.one_click,
                "verifiedUpdate": offer.verified_update,
                "externalUpdater": offer.external_updater,
                "installKind": offer.install_kind,
            }
        }));
    }

    Ok(json_result(CheckResult {
        status: CheckStatus::Available,
        current: current_text,
        release: Some(offer),
        error: None,
        error_detail: None,
        reboot_required,
    }))
}

fn json_result(r: CheckResult) -> Value {
    serde_json::to_value(r).unwrap_or(json!({"status": "error"}))
}

fn throttled(current: &str) -> CheckResult {
    CheckResult { status: CheckStatus::Throttled, current: current.into(), release: None, error: None, error_detail: None, reboot_required: false }
}

fn failed(current: &str, message: &'static str, detail: impl Into<String>) -> CheckResult {
    CheckResult {
        status: CheckStatus::Error,
        current: current.into(),
        release: None,
        error: Some(message.into()),
        error_detail: Some(detail.into()),
        reboot_required: false,
    }
}

fn install_kind(s: &Session, exe: &str) -> crate::update::InstallKind {
    detect_install_kind(exe, env_of(s))
}

fn download_and_install(
    s: &mut Session,
    latest_json: &[u8],
    release: &ReleaseInfo,
    kind: crate::update::InstallKind,
) -> Result<InstallOutcome, UpdateError> {
    let arch = release_arch_for(s);
    let file_name =
        pick_asset_name_with_arch(release.version, kind, &arch).ok_or_else(|| UpdateError::new(MSG_NO_INSTALLER, "no asset name for this kind"))?;
    let asset_url = crate::update::asset_download_url(latest_json, &file_name)
        .ok_or_else(|| UpdateError::new(MSG_NO_INSTALLER, format!("release does not list {file_name}")))?;
    let sums_url = checksums_download_url(latest_json).ok_or_else(|| UpdateError::new(MSG_NO_CHECKSUMS, "release has no SHA256SUMS asset"))?;
    let sums = http_get(s, &sums_url, MAX_JSON).map_err(|e| UpdateError::new(MSG_CHECKSUMS_UNREADABLE, e))?;
    let expected = checksum_for(&sums, &file_name)
        .map_err(|e| UpdateError::new(MSG_CHECKSUMS_UNREADABLE, e))?
        .ok_or_else(|| UpdateError::new(MSG_CHECKSUM_MISSING, format!("checksums do not list {file_name}")))?;
    let bytes = http_get(s, &asset_url, MAX_DOWNLOAD).map_err(|e| UpdateError::new(MSG_APPLY_FAILED, e))?;
    if !verify_sha256(&bytes, &expected) {
        return Err(UpdateError::new(MSG_CHECKSUM_MISMATCH, format!("sha256 mismatch for {file_name}")));
    }
    let install = s.update_install.as_ref().ok_or_else(|| UpdateError::new(MSG_NO_INSTALL_FN, "no installer hook"))?;
    let exe = s.update_exe.clone().unwrap_or_default();
    install(InstallHandoff { kind, file_name, bytes, exe_path: exe }).map_err(|e| UpdateError::new(MSG_APPLY_FAILED, e))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    fn test_now() -> u64 {
        2_000_000_000
    }
    use crate::update::{EnvRead, EnvReadBox, InstallHandoff, InstallKind, InstallOutcome, MAX_JSON};

    type TestHttp = Arc<dyn Fn(&str, usize) -> Result<Vec<u8>, String> + Send + Sync>;

    struct TestEnv {
        kind: InstallKind,
        no_update: bool,
    }

    impl EnvRead for TestEnv {
        fn flatpak(&self) -> bool {
            false
        }
        fn appimage(&self) -> Option<String> {
            None
        }
        fn install_override(&self) -> Option<InstallKind> {
            Some(self.kind)
        }
        fn windows(&self) -> bool {
            false
        }
        fn macos(&self) -> bool {
            false
        }
        fn linux(&self) -> bool {
            true
        }
        fn linux_deb_package(&self, _exe: &str) -> bool {
            self.kind == InstallKind::LinuxDeb
        }
        fn no_update_check(&self) -> bool {
            self.no_update
        }
    }

    fn session_with_kind(http: TestHttp, kind: InstallKind) -> Session {
        session_with_kind_arch(http, kind, "x86_64")
    }

    fn session_with_kind_arch(http: TestHttp, kind: InstallKind, arch: &str) -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.update_http = Some(http);
        s.update_exe = Some("/usr/bin/wordcraft".into());
        s.update_env = Some(Arc::new(TestEnv { kind, no_update: false }) as EnvReadBox);
        s.update_now_unix = test_now;
        s.update_release_arch = Some(arch.into());
        s
    }

    fn session_with(http: TestHttp) -> Session {
        session_with_kind(http, InstallKind::LinuxDeb)
    }

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(data);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A latest-release JSON that lists the checksum file and the asset for `kind`.
    fn latest_for(asset: &str) -> String {
        format!(
            r#"{{"tag_name":"v9.0.0","draft":false,"prerelease":false,"body":"Hi","html_url":"https://github.com/storytold/wordcraft/releases/tag/v9.0.0","assets":[{{"name":"SHA256SUMS.txt","browser_download_url":"https://example.com/sums"}},{{"name":"{asset}","browser_download_url":"https://example.com/pkg"}}]}}"#
        )
    }

    fn release_arch_for_kind(kind: InstallKind) -> &'static str {
        match kind {
            InstallKind::WindowsMsi => "x64",
            InstallKind::LinuxDeb => "x86_64",
            InstallKind::MacApp => "x86_64",
            _ => "x86_64",
        }
    }

    /// Run an apply check for `kind` with a healthy download and capture the handoff.
    fn apply_for(kind: InstallKind) -> (Value, Option<InstallHandoff>) {
        apply_for_arch(kind, release_arch_for_kind(kind))
    }

    fn apply_for_arch(kind: InstallKind, arch: &str) -> (Value, Option<InstallHandoff>) {
        let bytes = b"hello";
        let asset = pick_asset_name_with_arch((9, 0, 0), kind, arch).expect("asset for a supported install kind");
        let sums = format!("{}  {asset}\n", sha256_hex(bytes));
        let latest = latest_for(&asset);
        let got: Arc<Mutex<Option<InstallHandoff>>> = Arc::new(Mutex::new(None));
        let g = Arc::clone(&got);
        let mut s = session_with_kind_arch(
            Arc::new(move |u, _| {
                Ok(if u == LATEST_API {
                    latest.clone().into_bytes()
                } else if u.contains("sums") {
                    sums.clone().into_bytes()
                } else {
                    bytes.to_vec()
                })
            }),
            kind,
            arch,
        );
        s.update_install = Some(Arc::new(move |h: InstallHandoff| {
            *g.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
            Ok(InstallOutcome::default())
        }));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        let got = got.lock().unwrap_or_else(|e| e.into_inner()).clone();
        (r, got)
    }

    fn latest_linux_deb(arch: &str) -> String {
        latest_for(&format!("wordcraft-9.0.0-linux-{arch}.deb"))
    }

    #[test]
    fn no_newer_version() {
        let cur = env!("CARGO_PKG_VERSION");
        let tag = format!("v{cur}");
        let body = format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[]}}"#);
        let mut s = session_with(Arc::new(move |u, _| {
            assert_eq!(u, LATEST_API);
            Ok(body.clone().into_bytes())
        }));
        let r = check(&mut s, &json!({"force": true})).unwrap();
        assert_eq!(r["status"], "current");
    }

    #[test]
    fn newer_version_notice() {
        let latest = latest_linux_deb("x86_64");
        let s = session_with(Arc::new(move |_, _| Ok(latest.clone().into_bytes())));
        let mut s = s;
        let r = check(&mut s, &json!({"force": true})).unwrap();
        assert_eq!(r["status"], "available");
        assert!(s.ui_requests.iter().any(|x| x.get("update").is_some()));
        let up = s.ui_requests.iter().find_map(|x| x.get("update")).unwrap();
        assert_eq!(up.get("version").and_then(Value::as_str), Some("9.0.0"));
    }

    #[test]
    fn skip_this_version() {
        let latest = latest_linux_deb("x86_64");
        let s = session_with(Arc::new(move |_, _| Ok(latest.clone().into_bytes())));
        let mut s = s;
        s.update_prefs.skipped_version = "9.0.0".into();
        let r = check(&mut s, &json!({"force": true})).unwrap();
        assert_eq!(r["status"], "current");
    }

    #[test]
    fn skip_via_prefs_command() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        prefs(&mut s, &json!({"skipVersion": "1.2.3"})).unwrap();
        assert_eq!(s.update_prefs.skipped_version, "1.2.3");
    }

    #[test]
    fn once_a_day_throttle() {
        let calls = Arc::new(Mutex::new(0));
        let c = Arc::clone(&calls);
        let mut s = session_with(Arc::new(move |_, _| {
            *c.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(latest_linux_deb("x86_64").into_bytes())
        }));
        s.update_prefs.last_check_unix = 1_999_999_000;
        let r = check(&mut s, &json!({})).unwrap();
        assert_eq!(r["status"], "throttled");
        assert_eq!(*calls.lock().unwrap_or_else(|e| e.into_inner()), 0);
    }

    #[test]
    fn setting_off_means_no_request_on_startup() {
        let calls = Arc::new(Mutex::new(0));
        let c = Arc::clone(&calls);
        let mut s = session_with(Arc::new(move |_, _| {
            *c.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(vec![])
        }));
        s.update_prefs.check_on_startup = false;
        let r = check(&mut s, &json!({"startup": true})).unwrap();
        assert_eq!(r["status"], "throttled");
        assert_eq!(*calls.lock().unwrap_or_else(|e| e.into_inner()), 0);
    }

    #[test]
    fn checksum_mismatch_refused() {
        let latest = latest_linux_deb("x86_64");
        let sums = b"0000000000000000000000000000000000000000000000000000000000000000  wordcraft-9.0.0-linux-x86_64.deb\n";
        let mut s = session_with(Arc::new(move |u, _| {
            Ok(if u.contains("sums") {
                sums.to_vec()
            } else if u.contains("pkg") {
                b"bad".to_vec()
            } else {
                latest.clone().into_bytes()
            })
        }));
        s.update_install = Some(Arc::new(|_| Ok(InstallOutcome::default())));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "error");
        assert!(r["error"].as_str().unwrap_or("").to_ascii_lowercase().contains("checksum"));
    }

    #[test]
    fn large_asset_download_allowed() {
        let big = vec![b'a'; MAX_JSON + 64];
        let hash = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&big);
            let digest = h.finalize();
            digest.iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        let sums = format!("{hash}  wordcraft-9.0.0-linux-x86_64.deb\n");
        let installed = Arc::new(Mutex::new(false));
        let got = Arc::clone(&installed);
        let mut s = session_with(Arc::new(move |u, max| {
            Ok(if u.contains("sums") {
                assert_eq!(max, MAX_JSON);
                sums.clone().into_bytes()
            } else if u.contains("pkg") {
                assert_eq!(max, MAX_DOWNLOAD);
                big.clone()
            } else {
                assert_eq!(max, MAX_JSON);
                latest_linux_deb("x86_64").into_bytes()
            })
        }));
        let got2 = Arc::clone(&got);
        s.update_install = Some(Arc::new(move |_| {
            *got2.lock().unwrap_or_else(|e| e.into_inner()) = true;
            Ok(InstallOutcome::default())
        }));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "available");
        assert!(*got.lock().unwrap_or_else(|e| e.into_inner()));
    }

    #[test]
    fn every_supported_install_kind_hands_off() {
        for kind in [InstallKind::WindowsMsi, InstallKind::MacApp, InstallKind::LinuxDeb] {
            let arch = release_arch_for_kind(kind);
            let (r, got) = apply_for(kind);
            assert_eq!(r["status"], "available", "{kind:?}");
            let h = got.unwrap_or_else(|| panic!("no handoff for {kind:?}"));
            assert_eq!(h.kind, kind, "handoff kind");
            assert_eq!(Some(h.file_name), pick_asset_name_with_arch((9, 0, 0), kind, arch), "asset for {kind:?}");
            assert_eq!(h.bytes, b"hello", "verified bytes for {kind:?}");
        }
    }

    #[test]
    fn mac_apply_requests_manual_dmg_notice_not_quit() {
        let bytes = b"hello";
        let asset = pick_asset_name_with_arch((9, 0, 0), InstallKind::MacApp, "x86_64").unwrap();
        let sums = format!("{}  {asset}\n", sha256_hex(bytes));
        let latest = latest_for(&asset);
        let mut s = session_with_kind_arch(
            Arc::new(move |u, _| {
                Ok(if u == LATEST_API {
                    latest.clone().into_bytes()
                } else if u.contains("sums") {
                    sums.clone().into_bytes()
                } else {
                    bytes.to_vec()
                })
            }),
            InstallKind::MacApp,
            "x86_64",
        );
        s.update_install = Some(Arc::new(|_| Ok(InstallOutcome::default())));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "available");
        assert!(s.ui_requests.iter().any(|x| x.get("updateMacDmg").is_some()));
        assert!(!s.ui_requests.iter().any(|x| x.get("quit").and_then(Value::as_bool) == Some(true)));
        let offer = r.get("release").expect("release");
        assert_eq!(offer.get("oneClick").and_then(Value::as_bool), Some(false));
        assert_eq!(offer.get("verifiedUpdate").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn external_and_unknown_kinds_fall_back_to_release_page() {
        for kind in [InstallKind::Flatpak, InstallKind::AppImage, InstallKind::Unknown] {
            let latest = latest_linux_deb("x86_64");
            let mut s = session_with_kind(Arc::new(move |_, _| Ok(latest.clone().into_bytes())), kind);
            s.update_exe = Some("/tmp/dev/wordcraft".into());
            let err = check(&mut s, &json!({"force": true, "apply": true})).unwrap_err();
            assert!(err.to_string().to_ascii_lowercase().contains("release page"), "{kind:?}");
        }
    }

    #[test]
    fn missing_checksum_refused_with_catalogued_message() {
        let asset = pick_asset_name_with_arch((9, 0, 0), InstallKind::LinuxDeb, "x86_64").unwrap();
        let sums = format!("{}  some-other-file.deb\n", sha256_hex(b"hello"));
        let latest = latest_for(&asset);
        let called = Arc::new(Mutex::new(false));
        let c = Arc::clone(&called);
        let mut s = session_with(Arc::new(move |u, _| {
            Ok(if u == LATEST_API {
                latest.clone().into_bytes()
            } else if u.contains("sums") {
                sums.clone().into_bytes()
            } else {
                b"hello".to_vec()
            })
        }));
        s.update_install = Some(Arc::new(move |_| {
            *c.lock().unwrap_or_else(|e| e.into_inner()) = true;
            Ok(InstallOutcome::default())
        }));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "error");
        assert_eq!(r["error"], crate::update::MSG_CHECKSUM_MISSING);
        assert!(r["errorDetail"].as_str().is_some_and(|d| d.contains(&asset)));
        assert!(!*called.lock().unwrap_or_else(|e| e.into_inner()), "installer must not run");
    }

    #[test]
    fn no_update_check_skips_startup_request() {
        let calls = Arc::new(Mutex::new(0));
        let c = Arc::clone(&calls);
        let mut s = session_with(Arc::new(move |_, _| {
            *c.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(latest_linux_deb("x86_64").into_bytes())
        }));
        s.update_env = Some(Arc::new(TestEnv { kind: InstallKind::LinuxDeb, no_update: true }) as EnvReadBox);
        s.update_prefs.check_on_startup = true;
        let r = check(&mut s, &json!({"startup": true})).unwrap();
        assert_eq!(r["status"], "throttled");
        assert_eq!(*calls.lock().unwrap_or_else(|e| e.into_inner()), 0);
    }

    #[test]
    fn dirty_session_makes_no_request_and_never_invokes_installer() {
        let installs = Arc::new(Mutex::new(0));
        let requests = Arc::new(Mutex::new(0));
        let i = Arc::clone(&installs);
        let r = Arc::clone(&requests);
        let mut s = session_with(Arc::new(move |_, _| {
            *r.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(latest_linux_deb("x86_64").into_bytes())
        }));
        s.dirty = true;
        s.update_install = Some(Arc::new(move |_| {
            *i.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(InstallOutcome::default())
        }));
        let err = check(&mut s, &json!({"force": true, "apply": true})).unwrap_err();
        assert!(err.to_string().contains("unsaved"));
        assert_eq!(*installs.lock().unwrap_or_else(|e| e.into_inner()), 0);
        assert_eq!(*requests.lock().unwrap_or_else(|e| e.into_inner()), 0, "not even the release metadata is fetched before the save prompt");
    }

    #[test]
    fn linux_aarch64_asset_selected_independently_of_host() {
        let arch = "aarch64";
        let (r, got) = apply_for_arch(InstallKind::LinuxDeb, arch);
        assert_eq!(r["status"], "available");
        let h = got.unwrap();
        assert_eq!(h.file_name, format!("wordcraft-9.0.0-linux-{arch}.deb"));
    }

    #[test]
    fn windows_x64_msi_selected_independently_of_host() {
        let (r, got) = apply_for_arch(InstallKind::WindowsMsi, "x64");
        assert_eq!(r["status"], "available");
        let h = got.unwrap();
        assert_eq!(h.file_name, "wordcraft-9.0.0-windows-x64.msi");
    }

    /// MSI return code 3010 (installed, restart required) must count as success and be reported,
    /// not turned into an error that blocks the clean shutdown (#15).
    #[test]
    fn reboot_required_install_still_succeeds_and_is_reported() {
        let bytes = b"hello";
        let asset = pick_asset_name_with_arch((9, 0, 0), InstallKind::LinuxDeb, "x86_64").unwrap();
        let sums = format!("{}  {asset}\n", sha256_hex(bytes));
        let latest = latest_for(&asset);
        let mut s = session_with_kind_arch(
            Arc::new(move |u, _| {
                Ok(if u == LATEST_API {
                    latest.clone().into_bytes()
                } else if u.contains("sums") {
                    sums.clone().into_bytes()
                } else {
                    bytes.to_vec()
                })
            }),
            InstallKind::LinuxDeb,
            "x86_64",
        );
        s.update_install = Some(Arc::new(|_| Ok(InstallOutcome { reboot_required: true })));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "available", "a deferred restart is not a failure");
        assert_eq!(r["rebootRequired"], true, "the restart is reported to callers");
        assert!(s.ui_requests.iter().any(|x| x.get("updateReboot").is_some()), "the interface is told about the restart");
        assert!(
            s.ui_requests.iter().any(|x| x.get("quit").and_then(Value::as_bool) == Some(true)),
            "the app still quits to finish (after the save prompt)"
        );
    }

    #[test]
    fn hostile_json_never_panics() {
        let s = session_with(Arc::new(|_, max| Ok(vec![b'x'; max.saturating_add(1)])));
        let mut s = s;
        let r = check(&mut s, &json!({"force": true}));
        assert!(r.is_ok());
    }
}
