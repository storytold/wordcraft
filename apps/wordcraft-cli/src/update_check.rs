//! HTTPS for `wordcraft-cli update --check` (#15).

use std::sync::Arc;
use std::time::Duration;

use wordcraft_engine::update::{HttpGet, LATEST_API};

pub fn http_get() -> HttpGet {
    if let Ok(path) = std::env::var("WORDCRAFT_UPDATE_CHECK_FIXTURE") {
        let body = std::fs::read(&path).map_err(|e| e.to_string());
        return Arc::new(move |url, max_bytes| {
            if url != LATEST_API {
                return Err(format!("unexpected URL: {url}"));
            }
            let body = body.as_ref().map_err(|e| e.clone())?;
            let end = body.len().min(max_bytes);
            Ok(body[..end].to_vec())
        });
    }
    Arc::new(|url, max_bytes| {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("wordcraft-cli/", env!("CARGO_PKG_VERSION"), " (+https://github.com/storytold/wordcraft)"))
            .build()
            .into();
        let mut response = agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|e| e.to_string())?;
        response.body_mut().with_config().limit(max_bytes as u64).read_to_vec().map_err(|e| e.to_string())
    })
}
