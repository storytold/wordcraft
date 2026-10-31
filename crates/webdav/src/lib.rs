//! WebDAV client for one document: load it, lock it while it is open, save it back and unlock it.
//!
//! This is the desktop side of editing documents that live in a document management system or on
//! a WebDAV share, as Word and LibreOffice do it:
//!
//! 1. [`Resource::get`] loads the document (with a size cap).
//! 2. [`Resource::lock`] takes an exclusive write lock (RFC 4918 §9.10) and [`Resource::refresh`]
//!    keeps it alive, every [`Lock::refresh_interval`].
//! 3. [`Resource::put`] saves with the lock token in an `If` header, so a server that gave the lock
//!    to someone else refuses the save instead of losing their work.
//! 4. [`Resource::unlock`] releases the lock when the document closes.
//!
//! Every failure is an [`Error`]; nothing here panics. The client is native only: on wasm the crate
//! builds with just the shared types, because the browser build has no socket to talk to a server.
//!
//! Written from RFC 4918 (HTTP Extensions for WebDAV); no code from other WebDAV clients is used.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// The lock helpers serve the client, which wasm builds leave out.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod lock;

#[cfg(not(target_arch = "wasm32"))]
mod client;

#[cfg(test)]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
pub use client::Resource;
pub use lock::Lock;

/// What went wrong talking to a WebDAV server.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// The address isn't an `http://` or `https://` URL with a host.
    #[error("not a WebDAV address: {0}")]
    InvalidUrl(String),
    /// The server wants credentials (401) or refuses this request (403).
    #[error("the server refused {method} ({status}): no permission for this document")]
    Forbidden { method: &'static str, status: u16 },
    /// There is no document at this address (404, 410).
    #[error("the server has no document at this address ({status})")]
    NotFound { status: u16 },
    /// Someone else holds a lock on the document (423).
    #[error("the document is locked by someone else")]
    Locked,
    /// The lock this save relied on is gone or belongs to someone else (412).
    #[error("the server no longer holds this lock on the document, so it refused {method}")]
    LockLost { method: &'static str },
    /// The server answered with a status this request doesn't expect.
    #[error("the server answered {method} with {status}")]
    Status { method: &'static str, status: u16 },
    /// The document is larger than the limit the caller set.
    #[error("the document is larger than {limit} bytes")]
    TooLarge { limit: u64 },
    /// A LOCK succeeded but the answer carries no lock token to save or unlock with.
    #[error("the server granted the lock but sent no lock token")]
    NoLockToken,
    /// The connection failed, timed out, or the answer wasn't valid HTTP.
    #[error("could not reach the server: {0}")]
    Transport(String),
}

/// Result of a WebDAV operation.
pub type Result<T> = std::result::Result<T, Error>;
