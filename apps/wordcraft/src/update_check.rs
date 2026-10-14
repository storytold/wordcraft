//! HTTPS fetch for the daily update check (#15).

use std::sync::Arc;
use std::time::Duration;

use wordcraft_engine::update::HttpGet;

pub fn http_get() -> HttpGet {
    Arc::new(fetch_url)
}

fn fetch_url(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION"), " (+https://github.com/storytold/wordcraft)"))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(max_bytes as u64).read_to_vec().map_err(|e| e.to_string())
}
