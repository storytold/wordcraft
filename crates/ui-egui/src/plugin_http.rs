//! HTTPS for plug-in catalogs and GitHub releases (RFC-0046), over the client WordCraft checks for
//! updates with (its proxies and TLS settings). craft-plugin's own `http` feature implements this
//! for the craft-update of its craft-libs revision, which isn't the one WordCraft pins, so
//! WordCraft wraps its own client instead.

use craft_update::http::Fetched;
use wordcraft_engine::plugin::craft_plugin::Fetch;
use wordcraft_engine::plugin::craft_plugin::fetch::follow_redirects;

/// craft-update's client as craft-plugin's [`Fetch`].
struct UpdateHttp(craft_update::Http);

impl Fetch for UpdateHttp {
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        if url.starts_with("https://api.github.com/") {
            return match self.0.get_api(url, None)? {
                Fetched::Body { text, .. } if text.len() > max => Err(format!("GitHub's answer is larger than {max} bytes")),
                Fetched::Body { text, .. } => Ok(Some(text.into_bytes())),
                Fetched::NotFound => Ok(None),
                Fetched::RateLimited { .. } => Err("GitHub's API limit for this hour is used up: try again later".into()),
                Fetched::NotModified => Err("GitHub answered 304 Not Modified to a request without a cache".into()),
            };
        }
        // Every redirect stays on GitHub's hosts over HTTPS; a HEAD that fails ends the walk
        let end = follow_redirects(url, &mut |u| self.0.redirect_target(u).ok().flatten())?;
        self.0.get_bytes(&end, max)
    }
}

/// An HTTPS client for plug-in downloads, when one can be made: the session's
/// ([`wordcraft_engine::plugin::Runtime::set_fetch`]) and each download the Plug-ins window runs
/// on a worker thread.
pub fn http_fetch() -> Option<Box<dyn Fetch + Send>> {
    let agent = format!("WordCraft/{}", env!("CARGO_PKG_VERSION"));
    match craft_update::Http::new(&agent) {
        Ok(http) => Some(Box::new(UpdateHttp(http))),
        Err(e) => {
            log::warn!("plug-in downloads: {e}");
            None
        }
    }
}
