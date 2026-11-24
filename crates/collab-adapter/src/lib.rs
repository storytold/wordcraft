#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The WordCraft adapter between [`wordcraft_doc::Document`] and a live session.
//!
//! RFC-0020 puts the live-session contract in the shared layer and leaves each application an
//! adapter of its own: the app's document is edited through its own primitives, and the session
//! is reached only through a trait. This crate is that adapter half. It names no wire envelope, no
//! message kind and no encoding: a [`seam::Session`] carries the app's own [`edit::Edit`] values,
//! addresses the document by session-scoped block ids ([`ids`]) and orders submissions the way
//! RFC-0020 requires — one sequence, an acknowledgment that means "sequenced", and a refusal that
//! says why. What travels between two processes stays the shared contract's business; nothing here
//! guesses at it.
//!
//! The shared contract (RFC-0020 landing order item 2, the crate beside `craft-cloud`) does not
//! exist yet, so the trait is declared here as the seam the app writes against. When the contract
//! lands, its trait supersedes this one and the adapter keeps its document half unchanged; see
//! `docs/collab/adapter-increment.md`.
//!
//! [`relay::MockRelay`] is the in-process backend the conformance cases run against: it sequences
//! submissions, keeps the journal a reconnect replays, holds one role per peer, refuses a viewer
//! and answers presence latest-wins per peer. It is a test double, not a server: the relay binary
//! on the collaboration branch stays the source of the backend, and it implements the same trait
//! without the app knowing which one is in use.
//!
//! What the adapter edits, in this increment, is the body's top-level blocks: text inside a
//! paragraph, the removal of a paragraph and the insertion of one. Everything the branch's
//! `collab-doc` does beyond that (tables, parts, lists, formatting and their inverses) stays
//! app-side and moves behind the same seam in the next increment, not here.
//!
//! ## The five conformance cases
//!
//! `tests/conformance.rs` runs the cases `docs/collab/adapters.md` names for the first increment:
//! two clients converge and a late joiner catches up; a client reconnects and replays the journal
//! it missed; a structural removal and a text edit on the same scope have one outcome on every
//! replica and the typed text comes back to its author; a viewer is refused at the seam and at the
//! backend; a 100-page document keeps its pagination across the session and round-trips through
//! `.docx` together with a part the model does not read.

pub mod edit;
pub mod ids;
pub mod relay;
pub mod seam;
pub mod session;

pub use edit::{Edit, EditError, Effect, apply};
pub use ids::{BlockId, IdMap};
pub use relay::{MockClient, MockRelay};
pub use seam::{Accepted, Delivery, Join, Peer, Recovered, Refusal, RefusalCode, Role, Session};
pub use session::DocSession;
