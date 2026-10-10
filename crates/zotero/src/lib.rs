//! Zotero integration: WordCraft as a word processor Zotero can drive.
//!
//! Zotero's desktop app listens on `127.0.0.1:23116` for word-processor plugins (its published
//! "LibreOffice plugin wire protocol"). The word processor sends one integration command
//! ([`Command`]: add/edit citation, bibliography, refresh…); Zotero then shows its own dialogs
//! and drives the document through calls such as `Document_insertField` and `Field_setText`,
//! ending with `Document_complete`. [`client`] runs that exchange over a socket; [`Bridge`]
//! answers each call against an engine [`wordcraft_engine::Session`].
//!
//! Citations are stored the way Zotero's Word plugin stores them, so documents move freely
//! between WordCraft and Word: range fields whose code is `ADDIN ZOTERO_ITEM CSL_CITATION {…}` or
//! `ADDIN ZOTERO_BIBL {…} CSL_BIBLIOGRAPHY`, and document preferences in the custom properties
//! `ZOTERO_PREF_1…n`.
//!
//! Written from Zotero's public protocol documentation; no Zotero plugin code is used.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bridge;
pub mod client;
pub mod fields;
mod rich;
pub mod wire;

pub use bridge::{Bridge, Headless, Host};
pub use client::{Command, Outcome};
pub use wire::Call;

/// Errors talking to Zotero.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum ZoteroError {
    /// Nothing is listening: Zotero isn't running (or its integration is off).
    #[error("Zotero isn't running (start Zotero and try again): {0}")]
    NotRunning(String),
    /// The connection failed mid-session.
    #[error("connection to Zotero failed: {0}")]
    Io(String),
    /// Zotero sent something that isn't the protocol.
    #[error("unexpected data from Zotero: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests;
