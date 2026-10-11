//! "A new version is available" notice (#201).
//!
//! Desktop only, and only when the host provides [`Services::check_update`](crate::Services): at
//! most once a day, after the first frame, a background thread asks GitHub for the latest release.
//! If it is newer than this build, a slim bar under the ribbon offers Download (the release page
//! in the browser), Skip this version (remembered) and Dismiss (for this session). Nothing is
//! installed; File › Options › Check for updates turns the check off.
//!
//! The response is untrusted: its size is capped by the host, it is parsed defensively here, and
//! the Download button only ever opens a page of WordCraft's own GitHub releases.

use std::sync::mpsc::Receiver;

use egui::{Stroke, Ui, vec2};
use serde_json::Value;

use crate::WordApp;
use crate::theme::{Tokens, TypeRung};

/// Fetches the latest-release JSON (`/repos/storytold/wordcraft/releases/latest`). Runs on a
/// background thread; the host caps the size and time it takes.
pub type Fetch = std::sync::Arc<dyn Fn() -> Result<Vec<u8>, String> + Send + Sync>;

/// How long to wait between checks.
pub const INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Responses larger than this are ignored (the host should stop reading there too).
pub const MAX_RESPONSE: usize = 256 * 1024;

/// Every link the notice opens is a page under this prefix.
const RELEASES: &str = "https://github.com/storytold/wordcraft/releases/";

/// A release version: major, minor, patch.
pub type Version = (u64, u64, u64);

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release page to open.
    pub url: String,
}

impl Release {
    pub fn version_text(&self) -> String {
        let (a, b, c) = self.version;
        format!("{a}.{b}.{c}")
    }
}

/// The check's state for this run.
#[derive(Default)]
pub struct UpdateCheck {
    started: bool,
    rx: Option<Receiver<Result<Vec<u8>, String>>>,
    /// The newer release found, until the user acts on the notice.
    pub available: Option<Release>,
}

/// `v1.2.3`, `1.2.3` or `1.2` → a version; pre-releases (`1.2.3-rc.1`) and anything else → `None`.
/// Build metadata (`+…`) is ignored, as in semver.
pub fn parse_version(tag: &str) -> Option<Version> {
    if tag.len() > 64 {
        return None;
    }
    let tag = tag.trim();
    let tag = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let core = tag.split_once('+').map_or(tag, |(core, _)| core);
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

/// The version of this build.
pub fn current_version() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

/// Whether a check is due: enabled, and never checked or last checked a day or more ago (or in
/// the future, after the clock was set back). `now` and `last` are Unix seconds; `0` is unknown.
pub fn due(enabled: bool, last: u64, now: u64) -> bool {
    enabled && now != 0 && now.checked_sub(last).is_none_or(|elapsed| elapsed >= INTERVAL_SECS)
}

/// The release to offer, from GitHub's latest-release JSON: published (no draft or pre-release),
/// newer than `current` and not the version the user chose to skip. Junk gives `None`.
pub fn offer(json: &[u8], current: Version, skipped: &str) -> Option<Release> {
    if json.len() > MAX_RESPONSE {
        return None;
    }
    let v: Value = serde_json::from_slice(json).ok()?;
    if v.get("draft").and_then(Value::as_bool) != Some(false) || v.get("prerelease").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let version = parse_version(v.get("tag_name")?.as_str()?)?;
    if version <= current || parse_version(skipped) == Some(version) {
        return None;
    }
    // Only WordCraft's own release pages; anything else falls back to the releases list.
    let url = v
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|u| u.len() <= 256 && u.starts_with(RELEASES) && u.bytes().all(|b| b.is_ascii_graphic()))
        .map_or_else(|| format!("{RELEASES}latest"), str::to_string);
    Some(Release { version, url })
}

/// Unix seconds now (`0` when unknown, as on the web).
fn now_secs() -> u64 {
    let ms = crate::now_ms();
    if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as u64 } else { 0 }
}

/// Each frame: start the check when due (once per run, after the first frame) and collect its
/// answer.
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    if !app.update.started && app.fonts_frames >= 2 {
        let now = now_secs();
        if let Some(fetch) = app.services.check_update.clone()
            && due(app.ui.check_updates, app.ui.update_checked_at, now)
        {
            app.update.started = true;
            // Failed checks count too: GitHub isn't asked again until tomorrow.
            app.ui.update_checked_at = now;
            app.update.rx = spawn(fetch, ctx.clone());
        }
    }
    let Some(rx) = &app.update.rx else { return };
    match rx.try_recv() {
        Ok(Ok(json)) => {
            app.update.available = offer(&json, current_version(), &app.ui.skipped_update);
            match &app.update.available {
                Some(r) => log::info!("update check: WordCraft {} is available", r.version_text()),
                None => log::info!("update check: no newer release"),
            }
            app.update.rx = None;
        }
        Ok(Err(e)) => {
            log::info!("update check failed: {e}");
            app.update.rx = None;
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => app.update.rx = None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(fetch: Fetch, ctx: egui::Context) -> Option<Receiver<Result<Vec<u8>, String>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let r = std::thread::Builder::new().name("update-check".into()).spawn(move || {
        let _ = tx.send(fetch());
        ctx.request_repaint();
    });
    match r {
        Ok(_) => Some(rx),
        Err(e) => {
            log::info!("update check not started: {e}");
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn(_fetch: Fetch, _ctx: egui::Context) -> Option<Receiver<Result<Vec<u8>, String>>> {
    None
}

/// The notice bar under the ribbon, while a newer release is waiting.
pub fn bar(app: &mut WordApp, ui: &mut Ui) {
    if !app.ui.check_updates {
        return;
    }
    let Some(release) = app.update.available.clone() else { return };
    let t = Tokens::get(ui.ctx());
    let mut close = false;
    egui::Panel::top("update_notice")
        .exact_size(32.0)
        .frame(
            egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 0 }).stroke(Stroke::new(1.0, t.border)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                let text = crate::i18n::fmt(tl!("WordCraft {version} is available."), &[("version", &release.version_text())]);
                ui.label(egui::RichText::new(text).font(TypeRung::Control.medium()).color(t.text));
                let download = egui::Button::new(egui::RichText::new(tl!("Download")).color(t.on_accent)).fill(t.accent);
                if ui.add(download).on_hover_text(tl!("Open the release page in your browser")).clicked() {
                    app.canvas.open_url = Some(release.url.clone());
                    close = true;
                }
                if ui.button(tl!("Skip this version")).clicked() {
                    app.ui.skipped_update = release.version_text();
                    close = true;
                }
                if ui.button(tl!("Dismiss")).clicked() {
                    close = true;
                }
            });
        });
    if close {
        app.update.available = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_and_junk_tags_are_ignored() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert_eq!(parse_version("V2.0.1+build.7"), Some((2, 0, 1)));
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
        for junk in ["", "v", "latest", "1", "1.2.3.4", "1..2", "1.2.x", "v1.2.3-rc.1", "-1.2.3", "1.2.99999999999999999999999", &"9".repeat(100)] {
            assert_eq!(parse_version(junk), None, "{junk:?}");
        }
        let release = |tag: &str| {
            format!(
                r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"html_url":"https://github.com/storytold/wordcraft/releases/tag/{tag}"}}"#
            )
        };
        let now = (0, 4, 0);
        assert_eq!(
            offer(release("v0.5.0").as_bytes(), now, ""),
            Some(Release { version: (0, 5, 0), url: "https://github.com/storytold/wordcraft/releases/tag/v0.5.0".into() })
        );
        assert_eq!(offer(release("v0.4.0").as_bytes(), now, ""), None, "same version");
        assert_eq!(offer(release("v0.3.9").as_bytes(), now, ""), None, "older");
        assert_eq!(offer(release("nightly").as_bytes(), now, ""), None);
        assert_eq!(offer(release("v0.5.0").as_bytes(), now, "0.5.0"), None, "skipped");
        assert!(offer(release("v0.5.1").as_bytes(), now, "0.5.0").is_some(), "a later version than the skipped one");
        // Drafts, pre-releases, missing fields and junk JSON offer nothing.
        assert_eq!(offer(br#"{"tag_name":"v9.0.0","draft":true,"prerelease":false}"#, now, ""), None);
        assert_eq!(offer(br#"{"tag_name":"v9.0.0","draft":false,"prerelease":true}"#, now, ""), None);
        assert_eq!(offer(br#"{"tag_name":"v9.0.0"}"#, now, ""), None);
        assert_eq!(offer(br#"{"tag_name":9,"draft":false}"#, now, ""), None);
        assert_eq!(offer(b"<html>rate limited</html>", now, ""), None);
        assert_eq!(offer(&[b' '; MAX_RESPONSE + 1], now, ""), None);
        // A foreign link is never opened: the releases list is offered instead.
        let evil = br#"{"tag_name":"v9.0.0","draft":false,"prerelease":false,"html_url":"https://evil.example/wordcraft"}"#;
        assert_eq!(offer(evil, now, "").map(|r| r.url), Some("https://github.com/storytold/wordcraft/releases/latest".into()));
    }

    #[test]
    fn checks_at_most_once_a_day_and_only_when_enabled() {
        let now = 1_800_000_000;
        assert!(due(true, 0, now), "never checked");
        assert!(!due(false, 0, now), "turned off");
        assert!(!due(true, now - 60, now), "checked a minute ago");
        assert!(!due(true, now - INTERVAL_SECS + 1, now));
        assert!(due(true, now - INTERVAL_SECS, now), "a day ago");
        assert!(due(true, now + 3600, now), "clock set back");
        assert!(!due(true, 0, 0), "no clock");
    }
}
