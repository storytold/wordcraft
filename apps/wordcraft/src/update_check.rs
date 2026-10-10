//! The daily "new version available" check (#201): one HTTPS GET to GitHub's latest-release API,
//! with a short timeout and a size cap. No identifiers, cookies or telemetry are sent; the notice
//! itself is `wordcraft_ui_egui::update`.
//!
//! Off when `WORDCRAFT_NO_UPDATE_CHECK` is set (at run time, or at build time for packagers who
//! update WordCraft through their own channel), without preferences (`WORDCRAFT_NO_PREFS`: agents'
//! test runs), and in Flatpak, which updates apps itself and gives WordCraft no network access.

use std::time::Duration;

use wordcraft_ui_egui::update::{Fetch, MAX_RESPONSE};

const LATEST: &str = "https://api.github.com/repos/storytold/wordcraft/releases/latest";

/// Whether a value of `WORDCRAFT_NO_UPDATE_CHECK` turns the check off (anything but empty or `0`).
fn turned_off(v: Option<&str>) -> bool {
    v.is_some_and(|v| !v.trim().is_empty() && v.trim() != "0")
}

/// The fetcher for [`wordcraft_ui_egui::Services::check_update`], unless the check is off.
pub fn service(prefs_enabled: bool) -> Option<Fetch> {
    let env = std::env::var("WORDCRAFT_NO_UPDATE_CHECK").ok();
    if !prefs_enabled
        || turned_off(option_env!("WORDCRAFT_NO_UPDATE_CHECK"))
        || turned_off(env.as_deref())
        || std::env::var_os("FLATPAK_ID").is_some()
    {
        return None;
    }
    Some(std::sync::Arc::new(fetch))
}

fn fetch() -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION"), " (update check; +https://github.com/storytold/wordcraft)"))
        .build()
        .into();
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(MAX_RESPONSE as u64).read_to_vec().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_opt_out_variable_reads_like_a_switch() {
        for off in ["1", "true", "yes"] {
            assert!(super::turned_off(Some(off)), "{off}");
        }
        for on in [None, Some(""), Some("0"), Some(" 0 ")] {
            assert!(!super::turned_off(on), "{on:?}");
        }
    }
}
