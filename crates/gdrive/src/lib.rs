//! Google Drive for WordCraft: sign in with a Google account and open or save documents in Drive.
//!
//! - [`oauth`] signs in the way Google documents for installed apps: the system browser opens the
//!   consent page, Google redirects to a one-shot listener on `127.0.0.1` ([`loopback`]), and the
//!   code is exchanged with a PKCE verifier ([`pkce`], RFC 7636). No embedded web view.
//! - [`drive`] makes the Drive v3 calls: list documents, download (or export a Google Doc as
//!   `.docx`), create, and update with a conflict check against the file's `version`.
//! - [`store`] keeps the refresh token in the operating system's keychain, never in a file.
//! - [`http`] is the transport seam: [`http::Transport`] is implemented with `ureq` on native
//!   targets, and by fakes in tests, so no test touches the network or the keychain.
//!
//! The OAuth client comes from the person running WordCraft ([`oauth::ClientConfig`]); none is
//! built in. Everything here is blocking: callers run it on a worker thread, never on the typing
//! path.
//!
//! Written from Google's public OAuth 2.0 and Drive v3 REST documentation.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod drive;
pub mod encode;
pub mod http;
#[cfg(not(target_arch = "wasm32"))]
pub mod loopback;
pub mod oauth;
pub mod pkce;
pub mod store;

#[cfg(test)]
mod tests;

pub use drive::{Drive, DriveFile, SaveTarget};
pub use oauth::{Auth, ClientConfig, Tokens, write_client};

/// Errors from signing in or talking to Drive. Messages are English; the UI translates its own
/// summary and shows the detail as is.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// No OAuth client is configured.
    #[error("Google Drive isn't set up: no OAuth client ID ({0})")]
    NotConfigured(String),
    /// Not signed in, or the refresh token was revoked.
    #[error("not signed in to Google Drive")]
    NotSignedIn,
    /// The person closed the consent page, denied access, or sign-in timed out or was cancelled.
    #[error("sign-in didn't finish: {0}")]
    SignIn(String),
    /// The file changed in Drive since it was opened; nothing was overwritten.
    #[error("{name} was changed in Google Drive since you opened it (Drive version {remote}, opened {opened})")]
    Conflict { name: String, opened: String, remote: String },
    /// Drive answered with an error status.
    #[error("Google Drive returned {status}: {message}")]
    Api { status: u16, message: String },
    /// The response was malformed or larger than allowed.
    #[error("unexpected response from Google: {0}")]
    Protocol(String),
    /// A parameter was rejected before anything was sent (bad file id, empty name, too large).
    #[error("{0}")]
    Invalid(String),
    /// The network or TLS failed.
    #[error("couldn't reach Google: {0}")]
    Network(String),
    /// The keychain failed.
    #[error("keychain: {0}")]
    Keychain(String),
}

pub type Result<T> = std::result::Result<T, Error>;
