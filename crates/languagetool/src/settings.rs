//! The user's LanguageTool settings (File › Options › Proofing).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::LtError;

/// LanguageTool's public server.
pub const DEFAULT_SERVER: &str = "https://api.languagetool.org";

/// Longest server address accepted.
const MAX_SERVER_LEN: usize = 2048;
/// Longest username / API key kept.
const MAX_SECRET_LEN: usize = 512;

/// LanguageTool settings. Off by default: turning it on sends paragraph text to `server`.
///
/// `Debug` never shows the API key, and saved values that are junk read as the defaults rather
/// than discarding the rest of the preferences.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Check with LanguageTool.
    pub enabled: bool,
    /// The server's base address (`https://api.languagetool.org`, or a self-hosted
    /// `http://localhost:8081`); `/v2/check` is added.
    pub server: String,
    /// Premium account: the username (an email address) and API key. Both or neither are sent.
    pub username: String,
    pub api_key: String,
    /// `auto` (the server detects it), `document` (each paragraph's proofing language, else
    /// automatic) or a language code such as `en-US`.
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: false, server: DEFAULT_SERVER.into(), username: String::new(), api_key: String::new(), language: "auto".into() }
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("enabled", &self.enabled)
            .field("server", &self.server)
            .field("username", &self.username)
            .field("api_key", &if self.api_key.is_empty() { "" } else { "<set>" })
            .field("language", &self.language)
            .finish()
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let mut s = Settings::default();
        let text = |key: &str, max: usize| v.get(key).and_then(Value::as_str).filter(|t| t.len() <= max).map(str::to_string);
        if let Some(b) = v.get("enabled").and_then(Value::as_bool) {
            s.enabled = b;
        }
        if let Some(t) = text("server", MAX_SERVER_LEN) {
            s.server = t;
        }
        if let Some(t) = text("username", MAX_SECRET_LEN) {
            s.username = t;
        }
        if let Some(t) = text("apiKey", MAX_SECRET_LEN) {
            s.api_key = t;
        }
        if let Some(t) = text("language", 64) {
            s.language = t;
        }
        Ok(s)
    }
}

/// Is `code` shaped like a language code (`en`, `en-US`, `de-DE-x-simple-language`)?
pub fn is_language_code(code: &str) -> bool {
    wordcraft_proof::external::is_language_code(code)
}

impl Settings {
    /// The `/v2/check` address for [`Settings::server`]. An empty server is the public one; a
    /// trailing `/`, `/v2` or `/v2/check` is accepted.
    pub fn endpoint(&self) -> Result<String, LtError> {
        let server = self.server.trim();
        let server = if server.is_empty() { DEFAULT_SERVER } else { server };
        if server.len() > MAX_SERVER_LEN || server.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(LtError::BadServer(clip(server)));
        }
        let lower = server.to_ascii_lowercase();
        let rest = match lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) {
            Some(r) => r,
            None => return Err(LtError::BadServer(clip(server))),
        };
        if rest.starts_with(['/', '?', '#']) || rest.is_empty() || server.contains(['?', '#']) {
            return Err(LtError::BadServer(clip(server)));
        }
        // Credentials go in the request body: never over plain http to another machine.
        if lower.starts_with("http://") && self.has_account() && !is_loopback_authority(rest) {
            return Err(LtError::BadServer(clip(server)));
        }
        let mut base = server.trim_end_matches('/');
        for suffix in ["/v2/check", "/v2"] {
            if base.len() >= suffix.len() && base.get(base.len() - suffix.len()..).is_some_and(|t| t.eq_ignore_ascii_case(suffix)) {
                base = base.get(..base.len() - suffix.len()).unwrap_or(base).trim_end_matches('/');
                break;
            }
        }
        Ok(format!("{base}/v2/check"))
    }

    /// Whether the server is LanguageTool's hosted service (its rate limits apply).
    pub fn is_hosted(&self) -> bool {
        let Ok(url) = self.endpoint() else { return false };
        let lower = url.to_ascii_lowercase();
        let host = lower.split("://").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap_or("");
        host == "api.languagetool.org" || host == "api.languagetoolplus.com"
    }

    /// The language to ask for a paragraph whose proofing language is `para_lang`.
    pub fn language_for(&self, para_lang: Option<&str>) -> String {
        wordcraft_proof::external::language_for(&self.language, para_lang)
    }

    /// Whether a username and API key are both set (a premium account).
    pub fn has_account(&self) -> bool {
        !self.username.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

/// A short, safe excerpt of user input for an error message.
fn clip(s: &str) -> String {
    let mut out: String = s.chars().filter(|c| !c.is_control()).take(80).collect();
    if s.chars().count() > 80 {
        out.push('…');
    }
    out
}

/// Is the authority at the start of `rest` (`host[:port][/path]`) this machine? No user-info
/// (`127.0.0.1@elsewhere` is `elsewhere`), and an address only when it parses as loopback.
fn is_loopback_authority(rest: &str) -> bool {
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') || authority.contains('\\') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        let Some((addr, tail)) = v6.split_once(']') else { return false };
        if !(tail.is_empty() || tail.starts_with(':')) {
            return false;
        }
        return addr.parse::<std::net::Ipv6Addr>().is_ok_and(|a| a.is_loopback());
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host == "localhost" || host.ends_with(".localhost") || host.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}
