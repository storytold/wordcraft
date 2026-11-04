//! Shared editing for WordCraft.
//!
//! Temporarily disabled: the previous transport (`craft-collab`, a personal fork/repo) has been
//! removed rather than pinned to an unreviewed personal fork. [`Link::attach`] always returns
//! `None` until a generic, storytold-owned backend replaces it (see
//! storytold/craft-libs#160 and storytold/craft-libs#123); the app runs as an ordinary
//! single-writer word processor in the meantime. [`TokenSource`]/[`EnvToken`] carry no transport
//! dependency and are kept so the call site in `main.rs` does not need to change again once a
//! real backend lands.

use wordcraft_engine::Session as Engine;

/// Where the relay credential comes from.
///
/// A client never holds the signing secret. The relay verifies an HMAC that the service produced,
/// so all a client can do is present a token it was handed. Anything able to mint tokens could
/// forge any file id and any permission bits, and that is why the key stays server-side.
pub trait TokenSource {
    /// Where the relay is, and a signed token for one document.
    fn join(&self) -> Option<(String, String)>;
}

/// Reads the environment. A desktop login flow is a second implementation of the same trait
/// (`cloud::CloudToken`).
pub struct EnvToken;

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

impl TokenSource for EnvToken {
    fn join(&self) -> Option<(String, String)> {
        Some((env("COLLABCRAFT_RELAY")?, env("COLLABCRAFT_TOKEN")?))
    }
}

/// A live shared-editing link. Currently never constructed: [`Link::attach`] always returns
/// `None` with no transport wired in.
pub struct Link {
    /// Lines for the user, drained by the app each frame.
    pub notices: Vec<String>,
    /// Who else is in the document.
    pub roster: Vec<(String, String)>,
}

impl Link {
    /// Always returns `None`: shared editing has no backend right now. Shared editing is an
    /// addition, never a precondition for opening a file.
    pub fn attach(_engine: &mut Engine, _src: &dyn TokenSource) -> Option<Link> {
        None
    }

    /// One frame's work. Never called while shared editing has no backend (`attach` never
    /// returns a `Link`), kept so a future backend can fill it in without touching call sites.
    pub fn pump(&mut self, _engine: &mut Engine) {}
}
