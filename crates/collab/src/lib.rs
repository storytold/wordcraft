//! Co-editing (plugin protocol 2): the document as a shared Yjs document.
//!
//! WordCraft schema 1 (normative in `docs/collab.md`, shared with craft-collab's
//! `docs/schemas/wordcraft-1.md`) maps a [`Document`](wordcraft_doc::Document) onto Yjs shared
//! types: root `meta` `{app, schema, title}`; the main story `body` as an XmlFragment of `<p>` and
//! `<table>` elements, each paragraph's text a Yjs text with formatting as attributes and inline
//! objects as embeds; headers, footers, notes and text boxes (`headers`, `footers`, `notes`,
//! `textBoxes`: maps of stories), `comments`, `styles` (and `styleOrder`), `numbering`,
//! `settings`, `media` and `files`.
//!
//! A [`Replica`] keeps one such document in step with the engine's document:
//! [`Replica::seed`] shares a document, [`Replica::commit`] turns each committed command into one
//! transaction (typing becomes character inserts, formatting becomes Yjs `format`),
//! [`Replica::apply_update`] and [`Replica::refresh`] bring remote changes back into the model
//! (only the paragraphs that changed are read again), [`Replica::undo`] is the per-user undo, and
//! [`Replica::sticky`] / [`Replica::resolve`] turn positions into Yjs relative positions and back
//! (awareness, and the caret staying put while others type).
//!
//! [`crypto`] is the end-to-end encryption of section 4 of the protocol, [`awareness`] the
//! y-protocols awareness encoding. Nothing here knows about plugins, processes or the UI.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod awareness;
pub mod chat;
pub mod crypto;
pub mod json;
mod replica;
mod schema;

pub use replica::{Refreshed, Replica, canonical};

/// The wire format (`host.collab.formats`).
pub const FORMAT: &str = "yjs-v1";
/// This app in the shared document's `meta.app`.
pub const APP: &str = "wordcraft";
/// The document schema this build reads and writes (`meta.schema`).
pub const SCHEMA: u32 = 1;
/// Largest update sent or applied, before base64 (protocol 2, section 7).
pub const MAX_UPDATE: usize = 16 << 20;
/// Largest awareness update.
pub const MAX_AWARENESS: usize = 64 << 10;

/// Why a collab operation failed. Never a panic: a bad update is reported and dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollabError {
    /// The update (or state vector, or awareness message) can't be decoded or applied.
    Corrupt(String),
    /// The room holds another app's document, or another schema of ours.
    Mismatch {
        app: String,
        schema: u64,
    },
    /// Decryption failed: wrong key, tampering, or the message was meant for another room.
    Decrypt,
    /// Larger than the protocol allows.
    TooLarge(usize),
    /// The room's document hasn't arrived yet.
    NotReady,
    Other(String),
}

impl std::fmt::Display for CollabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollabError::Corrupt(m) => write!(f, "a damaged update was dropped: {m}"),
            CollabError::Mismatch { app, schema } => write!(f, "This room is for {app} {schema}"),
            CollabError::Decrypt => write!(f, "a message that can't be decrypted was dropped"),
            CollabError::TooLarge(n) => write!(f, "a message of {n} bytes is larger than the 16 MiB allowed"),
            CollabError::NotReady => write!(f, "the room's document hasn't arrived yet"),
            CollabError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CollabError {}

/// Standard base64 with padding (protocol 2's binary payloads).
pub fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decodes standard base64 (padding optional).
pub fn b64_decode(text: &str) -> Result<Vec<u8>, CollabError> {
    use base64::Engine;
    let t = text.trim();
    base64::engine::general_purpose::STANDARD
        .decode(t)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t.trim_end_matches('=')))
        .map_err(|e| CollabError::Corrupt(format!("base64: {e}")))
}

/// Fills `buf` with random bytes from the operating system. The browser build has no source
/// here (and no transports): it fails there.
pub fn random_bytes(buf: &mut [u8]) -> Result<(), CollabError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        getrandom::fill(buf).map_err(|e| CollabError::Other(format!("no random numbers: {e}")))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = buf;
        Err(CollabError::Other("live sessions need the desktop app".into()))
    }
}

/// A fresh Yjs client id: random, below 2^53 (JavaScript-safe), never 0.
pub fn new_client_id() -> u64 {
    let mut b = [0u8; 8];
    let v = if random_bytes(&mut b).is_ok() { u64::from_le_bytes(b) } else { yrs::block::ClientID::random().get() };
    (v & ((1u64 << 53) - 1)).max(1)
}

/// The first id of the block of 65,536 story and comment ids a client creates new ones in
/// (schema 1, "Ids"): 65,536 × (1 + clientId mod 32,767). Ids below 65,536 are the ones the
/// document had when it was shared, so a room's peers never pick the same new id.
pub fn id_space(client_id: u64) -> u32 {
    ((client_id % 32_767) as u32 + 1) << 16
}

#[cfg(test)]
mod tests;
