//! The `POST /v2/check` request for one paragraph.

use crate::{LtError, Settings};

/// The language value that asks the server to detect the language.
pub const AUTO: &str = "auto";

/// The most text one request carries (the public server's limit for free use is 20 KB).
pub const MAX_TEXT_BYTES: usize = 20_000;

/// A request ready to send: `body` is form-encoded (`application/x-www-form-urlencoded`).
/// `Debug` leaves the body out: it holds the user's text and API key.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub body: String,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("url", &self.url).field("body_bytes", &self.body.len()).finish()
    }
}

/// The request checking `text` in `language` (`auto` or a code; anything else is sent as `auto`).
pub fn build(settings: &Settings, text: &str, language: &str) -> Result<Request, LtError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(LtError::TooLong { bytes: text.len(), max: MAX_TEXT_BYTES });
    }
    let url = settings.endpoint()?;
    let language = if language == AUTO || crate::settings::is_language_code(language) { language } else { AUTO };
    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("text", text);
    form.append_pair("language", language);
    if settings.has_account() {
        form.append_pair("username", settings.username.trim());
        form.append_pair("apiKey", settings.api_key.trim());
    }
    Ok(Request { url, body: form.finish() })
}

/// The fields of a form-encoded body (tests and diagnostics; never logged).
pub fn fields(body: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes()).into_owned().collect()
}
