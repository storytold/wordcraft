//! Collaboration protocol for WordCraft.
//!
//! Transport-independent half of the sync server: the wire envelope
//! ([`wire`]), the version journal with `opId` deduplication ([`journal`]),
//! session-scoped node identifiers and tombstones ([`ids`]), and per-user undo
//! stacks ([`undo`]).
//!
//! The decisions behind these types are `bus/decisions/0001…0010`:
//! an operation is a normalized intent applied by the server (ADR-0001),
//! identity is session-scoped (ADR-0002), one server `version` and `opId`
//! dedup (ADR-0003), `merge`/`structural` classes (ADR-0004), snapshots are the
//! model plus the source file (ADR-0005), presence is ephemeral (ADR-0006),
//! one control plane with per-app adapters (ADR-0007), JWT plus lease
//! (ADR-0008), the server commits (ADR-0009), undo is per user (ADR-0010).
//!
//! Nothing here performs I/O or talks to a network: the server binary wires
//! these types to `tokio`/`axum`, and tests drive them directly.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod ids;
pub mod journal;
pub mod room;
pub mod undo;
pub mod wire;

pub use ids::{NodeId, NodeIds};
pub use journal::{DedupWindow, Entry, Journal};
pub use room::{Effect, JoinPlan, Model, Role, Room};
pub use undo::{UndoStacks, UndoStep};
pub use wire::{Applied, ClientMsg, NodeRef, Op, OpId, OpKind, Reject, RejectCode, ServerMsg, Target, Version};

#[cfg(test)]
mod tests;
