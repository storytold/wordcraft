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
    (2..=35).contains(&code.len())
        && code.starts_with(|c: char| c.is_ascii_alphabetic())
        && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !code.ends_with('-')
}

impl Settings {
    /// The `/v2/check` address for [`Settings::server`]. An empty server is the public one; a
    /// trailing `/`, `/v2` or `/v2/check` is accepted.
    pub fn endpoint(&self) -> Result<String, LtError> {
        let server = self.server.trim();
        let server = if server.is_empty() { DEFAULT_SERVER } else { server };
        if server.len() > MAX_SERVER_LEN || server.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
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
        let chosen = self.language.trim();
        let code = match chosen {
            "" | "auto" => None,
            "document" => para_lang.map(str::trim),
            c => Some(c),
        };
        code.filter(|c| is_language_code(c)).map(str::to_string).unwrap_or_else(|| crate::request::AUTO.to_string())
    }

    /// Whether the username and API key may go to the server: over `https://`, or over `http://`
    /// only to this computer (`localhost`, `127.0.0.0/8`, `::1`), where nobody on the network can
    /// read them. Settings without an account always pass.
    pub fn account_check(&self) -> Result<(), LtError> {
        if !self.has_account() {
            return Ok(());
        }
        let url = self.endpoint()?;
        let lower = url.to_ascii_lowercase();
        match lower.strip_prefix("http://") {
            Some(rest) if !is_loopback_host(rest) => Err(LtError::InsecureAccount),
            _ => Ok(()),
        }
    }

    /// Whether a username and API key are both set (a premium account).
    pub fn has_account(&self) -> bool {
        !self.username.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

/// Whether the authority at the start of `rest` (what follows `scheme://`) names this computer.
fn is_loopback_host(rest: &str) -> bool {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host_port.split(':').next().unwrap_or(""),
    };
    let host = host.trim_end_matches('.');
    host == "localhost" || host.ends_with(".localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// A short, safe excerpt of user input for an error message.
fn clip(s: &str) -> String {
    let mut out: String = s.chars().filter(|c| !c.is_control()).take(80).collect();
    if s.chars().count() > 80 {
        out.push('…');
    }
    out
}
