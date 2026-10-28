//! Co-editing for WordCraft: [`macula-craft-seam`]'s shared-document session
//! over WordCraft's own document and its own command bus.
//!
//! The seam owns everything that is the same in every app — signed ops,
//! grants, invites, the CRDT body, presence, the protocol. An app supplies two
//! things and nothing else: how to read and write a paragraph by its stable id
//! ([`adapter`]), and how to run one of its commands ([`host`], through the
//! allowlist in [`shareable`]).
//!
//! Behind the `collab` feature, off by default: a plain build has no mesh crate
//! in its graph at all, and the web build must never enable it (`macula-rust`
//! pulls `aws-lc-rs` and `keyring`, which have no wasm story).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod shareable;

pub use shareable::{REFUSED_IN_FAMILY, SHAREABLE};
