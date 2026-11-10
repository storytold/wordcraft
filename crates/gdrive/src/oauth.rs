//! OAuth 2.0 for installed apps, as Google documents it: authorization code with PKCE, the
//! system browser, and a loopback redirect to `http://127.0.0.1:<port>`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::encode::{self, form};
use crate::http::{Method, Request, Transport};
use crate::store::TokenStore;
use crate::{Error, Result};

pub const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";

/// Full Drive access, so documents that WordCraft didn't create can be listed and opened.
pub const SCOPE_DRIVE: &str = "https://www.googleapis.com/auth/drive";
/// Only files WordCraft created or was given: the narrower choice (`"scope": "file"`).
pub const SCOPE_DRIVE_FILE: &str = "https://www.googleapis.com/auth/drive.file";

/// The settings file in the profile folder that names the OAuth client.
pub const CONFIG_FILE: &str = "google-drive.json";
pub const ENV_CLIENT_ID: &str = "WORDCRAFT_GOOGLE_CLIENT_ID";
pub const ENV_CLIENT_SECRET: &str = "WORDCRAFT_GOOGLE_CLIENT_SECRET";

/// Refresh an access token this long before Google says it expires.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
/// Longest token we accept from Google (they are ~200–2000 bytes).
const MAX_TOKEN_LEN: usize = 8192;

/// The OAuth client: a "Desktop app" client ID from the person's own Google Cloud project.
/// Google calls the secret of an installed-app client non-confidential, but it is still kept
/// out of logs and out of the repository.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scope: String,
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "…"))
            .field("scope", &self.scope)
            .finish()
    }
}

impl ClientConfig {
    pub fn new(client_id: &str, client_secret: Option<&str>, scope: Option<&str>) -> Result<Self> {
        let client_id = client_id.trim();
        let valid = |s: &str| s.len() <= 512 && s.bytes().all(|b| b.is_ascii_graphic());
        if client_id.is_empty() || !valid(client_id) {
            return Err(Error::NotConfigured("the client ID is empty or malformed".into()));
        }
        let client_secret = client_secret.map(str::trim).filter(|s| !s.is_empty());
        if client_secret.is_some_and(|s| !valid(s)) {
            return Err(Error::NotConfigured("the client secret is malformed".into()));
        }
        let scope = match scope.map(str::trim) {
            None | Some("" | "drive") => SCOPE_DRIVE,
            Some("file" | "drive.file") => SCOPE_DRIVE_FILE,
            Some(other) => return Err(Error::NotConfigured(format!("unknown scope {other:?}; use \"drive\" or \"file\""))),
        };
        Ok(Self { client_id: client_id.to_owned(), client_secret: client_secret.map(str::to_owned), scope: scope.to_owned() })
    }

    /// From `WORDCRAFT_GOOGLE_CLIENT_ID` (and `…_SECRET`), else `google-drive.json` in `dir`:
    /// `{"client_id": "…", "client_secret": "…", "scope": "drive" | "file"}`.
    pub fn load(dir: Option<&Path>) -> Result<Self> {
        if let Ok(id) = std::env::var(ENV_CLIENT_ID) {
            let secret = std::env::var(ENV_CLIENT_SECRET).ok();
            return Self::new(&id, secret.as_deref(), None);
        }
        let Some(dir) = dir else {
            return Err(Error::NotConfigured(format!("set {ENV_CLIENT_ID}")));
        };
        let path = dir.join(CONFIG_FILE);
        let text = match std::fs::read(&path) {
            Ok(b) if b.len() <= 64 * 1024 => b,
            Ok(_) => return Err(Error::NotConfigured(format!("{} is too large", path.display()))),
            Err(_) => return Err(Error::NotConfigured(format!("add {} or set {ENV_CLIENT_ID}", path.display()))),
        };
        Self::from_json(&text).map_err(|e| match e {
            Error::NotConfigured(m) => Error::NotConfigured(format!("{}: {m}", path.display())),
            e => e,
        })
    }

    /// Parses the settings file. Also accepts the JSON Google Cloud Console downloads for a
    /// Desktop client (`{"installed": {"client_id": …, "client_secret": …}}`).
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| Error::NotConfigured(format!("not valid JSON: {e}")))?;
        let o = v.get("installed").unwrap_or(&v);
        let s = |k: &str| o.get(k).and_then(|x| x.as_str());
        let id = s("client_id").ok_or_else(|| Error::NotConfigured("no \"client_id\"".into()))?;
        Self::new(id, s("client_secret"), v.get("scope").and_then(|x| x.as_str()))
    }
}

/// The folder a browser saves downloads to: `~/Downloads` (`%USERPROFILE%\\Downloads` on Windows).
pub fn downloads_dir() -> Option<std::path::PathBuf> {
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    home.filter(|h| !h.is_empty()).map(|h| std::path::PathBuf::from(h).join("Downloads"))
}

/// The newest `client_secret_*.json` in `dir`: the name Google Cloud console gives a downloaded
/// OAuth client. `None` when there is none.
pub fn newest_client_file(dir: &Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("client_secret_") && n.ends_with(".json")))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// Imports a client JSON downloaded from Google Cloud console (`{"installed": {…}}`) into
/// `dir`'s `google-drive.json`: checked first, so a wrong file changes nothing.
pub fn import_client(dir: &Path, file: &Path) -> Result<ClientConfig> {
    let text = match std::fs::read(file) {
        Ok(b) if b.len() <= 64 * 1024 => b,
        Ok(_) => return Err(Error::NotConfigured(format!("{} is too large for an OAuth client file", file.display()))),
        Err(e) => return Err(Error::NotConfigured(format!("couldn't read {}: {e}", file.display()))),
    };
    let cfg = ClientConfig::from_json(&text)?;
    write_client(dir, &cfg.client_id, cfg.client_secret.as_deref())?;
    Ok(cfg)
}

/// Writes `google-drive.json` in `dir` (File › Options › Accounts): the client ID and secret,
/// checked first, keeping a `scope` the file already had. Owner-only on Unix.
pub fn write_client(dir: &Path, client_id: &str, client_secret: Option<&str>) -> Result<()> {
    let path = dir.join(CONFIG_FILE);
    let scope = std::fs::read(&path)
        .ok()
        .filter(|b| b.len() <= 64 * 1024)
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("scope").and_then(|x| x.as_str()).map(str::to_string));
    let cfg = ClientConfig::new(client_id, client_secret, scope.as_deref())?;
    let mut v = serde_json::json!({ "client_id": cfg.client_id });
    if let Some(secret) = &cfg.client_secret {
        v["client_secret"] = secret.as_str().into();
    }
    if let Some(scope) = scope {
        v["scope"] = scope.into();
    }
    let text = serde_json::to_vec_pretty(&v).map_err(|e| Error::Protocol(e.to_string()))?;
    let io = |e: std::io::Error| Error::NotConfigured(format!("couldn't write {}: {e}", path.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(io)?;
    // `mode` only applies to a new file: an older one made by hand becomes owner-only too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    }
    std::io::Write::write_all(&mut file, &text).map_err(io)
}

/// The consent page URL. `redirect` is the loopback address that will receive the code.
pub fn auth_url(cfg: &ClientConfig, redirect: &str, challenge: &str, state: &str) -> String {
    let q = form(&[
        ("client_id", &cfg.client_id),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", &cfg.scope),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        // A refresh token, and asked for every time so one comes back on a second sign-in.
        ("access_type", "offline"),
        ("prompt", "consent"),
    ]);
    format!("{AUTH_ENDPOINT}?{q}")
}

/// Reads the authorization code out of the redirect's request target (`/?code=…&state=…`),
/// checking `state`. Returns `Ok(None)` for a request that isn't the redirect (a favicon).
pub fn parse_callback(target: &str, expected_state: &str) -> Result<Option<String>> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return Ok(None);
    }
    let pairs = encode::parse_query(query, 32);
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if let Some(err) = get("error") {
        return Err(Error::SignIn(match err {
            "access_denied" => "access was denied on the Google consent page".into(),
            e => format!("Google returned {e:?}"),
        }));
    }
    // Compare state before looking at the code: a request from anything but our consent page is
    // refused.
    if get("state") != Some(expected_state) {
        return Err(Error::SignIn("the sign-in response didn't match this request".into()));
    }
    match get("code") {
        Some(c) if !c.is_empty() && c.len() <= MAX_TOKEN_LEN => Ok(Some(c.to_owned())),
        _ => Err(Error::SignIn("Google sent no authorization code".into())),
    }
}

/// Tokens for one account. Only the refresh token is stored (in the keychain).
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Instant,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// Parses a token endpoint response received at `now`.
pub fn parse_tokens(v: &serde_json::Value, now: Instant) -> Result<Tokens> {
    let tok =
        |k: &str| -> Option<String> { v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty() && s.len() <= MAX_TOKEN_LEN).map(str::to_owned) };
    let access_token = tok("access_token").ok_or_else(|| Error::Protocol("no access_token".into()))?;
    // Google says 3599; a missing or absurd value is treated as short-lived.
    let secs = v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(0).min(86_400);
    let expires_at = now.checked_add(Duration::from_secs(secs)).unwrap_or(now);
    Ok(Tokens { access_token, refresh_token: tok("refresh_token"), expires_at })
}

/// A signed-in (or signed-out) account: the client, the transport, the keychain and the
/// current tokens. Not `Clone`: one per app, owned by the worker thread.
pub struct Auth {
    cfg: ClientConfig,
    http: Arc<dyn Transport>,
    store: Box<dyn TokenStore>,
    tokens: Option<Tokens>,
}

impl Auth {
    pub fn new(cfg: ClientConfig, http: Arc<dyn Transport>, store: Box<dyn TokenStore>) -> Self {
        Self { cfg, http, store, tokens: None }
    }

    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }
    pub fn http(&self) -> &Arc<dyn Transport> {
        &self.http
    }

    /// Picks up a refresh token stored by an earlier session. No network.
    pub fn restore(&mut self) -> Result<bool> {
        if self.tokens.is_some() {
            return Ok(true);
        }
        Ok(match self.store.load()? {
            Some(refresh) => {
                // Expired at once, so the first call refreshes.
                self.tokens = Some(Tokens { access_token: String::new(), refresh_token: Some(refresh), expires_at: Instant::now() });
                true
            }
            None => false,
        })
    }

    pub fn signed_in(&self) -> bool {
        self.tokens.is_some()
    }

    fn token_request<'a>(&'a self, mut pairs: Vec<(&'a str, &'a str)>) -> Result<serde_json::Value> {
        pairs.push(("client_id", &self.cfg.client_id));
        if let Some(s) = &self.cfg.client_secret {
            pairs.push(("client_secret", s));
        }
        let req = Request::new(Method::Post, TOKEN_ENDPOINT).body("application/x-www-form-urlencoded", form(&pairs).into_bytes());
        self.http.send(req)?.json()
    }

    /// Exchanges an authorization code (with its PKCE verifier and the exact redirect URI used)
    /// and stores the refresh token.
    pub fn exchange(&mut self, code: &str, verifier: &str, redirect: &str, now: Instant) -> Result<()> {
        let v =
            self.token_request(vec![("grant_type", "authorization_code"), ("code", code), ("code_verifier", verifier), ("redirect_uri", redirect)])?;
        let tokens = parse_tokens(&v, now)?;
        match &tokens.refresh_token {
            Some(r) => {
                if let Err(e) = self.store.save(r) {
                    // Signed in for this session only; say so, don't fail the sign-in.
                    log::warn!("Google Drive: refresh token not kept: {e}");
                }
            }
            None => log::warn!("Google Drive: no refresh token returned; sign-in lasts this session"),
        }
        self.tokens = Some(tokens);
        Ok(())
    }

    /// A valid access token, refreshing it first when it is about to expire.
    pub fn access_token(&mut self, now: Instant) -> Result<String> {
        let Some(t) = &self.tokens else { return Err(Error::NotSignedIn) };
        if !t.access_token.is_empty() && now + EXPIRY_MARGIN < t.expires_at {
            return Ok(t.access_token.clone());
        }
        let Some(refresh) = t.refresh_token.clone() else {
            self.tokens = None;
            return Err(Error::NotSignedIn);
        };
        let v = match self.token_request(vec![("grant_type", "refresh_token"), ("refresh_token", &refresh)]) {
            Ok(v) => v,
            // Revoked or expired: forget it so the UI offers Sign in again.
            Err(Error::Api { status: 400 | 401, message }) => {
                log::info!("Google Drive: refresh refused ({message}); signing out");
                self.forget();
                return Err(Error::NotSignedIn);
            }
            Err(e) => return Err(e),
        };
        let mut fresh = parse_tokens(&v, now)?;
        // Google usually omits the refresh token on refresh; keep the one we have.
        if fresh.refresh_token.is_none() {
            fresh.refresh_token = Some(refresh);
        }
        let token = fresh.access_token.clone();
        self.tokens = Some(fresh);
        Ok(token)
    }

    /// Drops the cached access token (Drive answered 401), so the next call refreshes it.
    pub fn invalidate_access(&mut self) {
        if let Some(t) = &mut self.tokens {
            t.access_token.clear();
        }
    }

    fn forget(&mut self) {
        self.tokens = None;
        if let Err(e) = self.store.clear() {
            log::warn!("Google Drive: couldn't remove the stored token: {e}");
        }
    }

    /// Revokes the grant at Google (best effort) and removes the stored token.
    pub fn sign_out(&mut self) -> Result<()> {
        let token = self.tokens.as_ref().and_then(|t| t.refresh_token.clone().or(Some(t.access_token.clone())));
        if let Some(token) = token.filter(|t| !t.is_empty()) {
            let req = Request::new(Method::Post, REVOKE_ENDPOINT).body("application/x-www-form-urlencoded", form(&[("token", &token)]).into_bytes());
            if let Err(e) = self.http.send(req) {
                log::info!("Google Drive: revoke failed ({e}); removing the local token anyway");
            }
        }
        self.tokens = None;
        self.store.clear()
    }

    /// Signs in through the system browser: binds the loopback listener, hands the consent URL
    /// to `open_browser`, waits for Google's redirect until `cancel` is set or `timeout`
    /// passes, then exchanges the code.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sign_in(&mut self, open_browser: &dyn Fn(&str), cancel: &std::sync::atomic::AtomicBool, timeout: Duration) -> Result<()> {
        let pkce = crate::pkce::Pkce::generate()?;
        let state = crate::pkce::random_token(24)?;
        let listener = crate::loopback::Listener::bind()?;
        let redirect = listener.redirect_uri();
        open_browser(&auth_url(&self.cfg, &redirect, &pkce.challenge, &state));
        let code = listener.wait(&state, cancel, timeout)?;
        self.exchange(&code, &pkce.verifier, &redirect, Instant::now())
    }
}
