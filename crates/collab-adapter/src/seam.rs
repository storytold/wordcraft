//! The session seam: what the adapter needs from a live session, and nothing more.
//!
//! RFC-0020 makes the shared layer's trait the only interface an application may depend on. That
//! contract (its landing order puts it beside `craft-cloud`) does not exist yet, so this module
//! declares the seam the app writes against and that a backend implements. It speaks the RFC's
//! vocabulary — a join sequence, ordered submission, an acknowledgment that means "sequenced", a
//! refusal that says why, presence latest-wins per peer — and it carries no envelope, no message
//! kind and no encoding: the payload is the application's own [`Edit`]. A backend may put anything
//! it likes on the wire; the app never sees it.
//!
//! When the shared contract lands, its trait replaces [`Session`] and a thin `impl` delegates to
//! the backend below it. The document half ([`crate::edit`], [`crate::ids`], [`crate::session`])
//! does not change, which is the point of doing the seam first.

use crate::edit::Edit;
use crate::ids::BlockId;

/// What a peer may do in a session. The relay grants it (it comes from a token in a deployment)
/// and the adapter gates on it as well, so a viewer is refused even before a submission leaves the
/// process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Reads and shows presence, changes nothing.
    Viewer,
    /// Edits.
    Editor,
    /// Edits and owns the document; the cabinet commit is the owner's (or the cabinet's) act.
    Owner,
}

impl Role {
    /// Whether the role may submit an edit.
    pub fn may_edit(self) -> bool {
        !matches!(self, Role::Viewer)
    }
}

/// Somebody else in the document right now.
///
/// Ephemeral and latest-wins: the position replaces the previous one from the same peer, is never
/// journalled and never enters the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// The session's own identifier, stable while it is connected.
    pub id: String,
    pub role: Role,
    /// Where they are working, when the session says. A word processor knows the paragraph.
    pub at: Option<String>,
}

/// Why a submission was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalCode {
    /// The peer's role does not allow the edit.
    ReadOnly,
    /// The submission was built on a version behind a change of the same paragraph: retry after
    /// syncing. Nothing was sequenced.
    Stale,
    /// The block is gone: a structural edit sequenced first removed it. The text comes back to its
    /// author through [`Recovered`].
    Gone,
    /// The edit addresses something the adapter does not model.
    Unsupported,
    /// The edit is malformed for its kind (an offset inside a character, a removal of the last
    /// paragraph, an id already in use).
    BadRequest,
    /// The session has no replica number left to give the peer. Nothing was joined: a replica
    /// identity is granted, never borrowed, so a peer is refused rather than sharing a slice.
    NoReplica,
    /// A replica cannot be brought up to date from the journal it asked for; it must resync from
    /// the document.
    Resync,
}

/// A refusal with the reason in words, suitable for the panel and the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: RefusalCode,
    pub detail: String,
}

impl Refusal {
    pub fn new(code: RefusalCode, detail: impl Into<String>) -> Refusal {
        Refusal { code, detail: detail.into() }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.detail)
    }
}

/// The relay's answer to an accepted submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The sequence number the operation was given. It is durable once the relay has answered
    /// with it (RFC-0020, mixed editing and recovery, guarantee 2).
    pub version: u64,
}

/// An operation the relay sequenced, in the order every replica applies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub version: u64,
    /// The peer whose submission it was. The relay supplies it; a submission never names its
    /// author.
    pub author: String,
    pub edit: Edit,
}

/// The state a joining peer receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    /// The relay's sequence state. A joiner reconciles with the persistent counter, never with a
    /// fresh zero (RFC-0020, constraint 2).
    pub version: u64,
    /// The identity space the relay granted this peer for the blocks it creates (constraint 3).
    pub replica: u8,
    /// The operations after the `since` the peer asked for, in order.
    pub replay: Vec<Delivery>,
    /// Who is in the document now.
    pub peers: Vec<Peer>,
}

/// Text an acknowledged structural edit orphaned, handed back to the peer that typed it.
///
/// RFC-0020's first recovery guarantee: the structural edit wins visibility, every replica drops
/// the text the same way, and the author keeps a recoverable copy — somebody's typing never just
/// disappears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub block: BlockId,
    pub text: String,
    /// What removed the scope, in words for the panel.
    pub detail: String,
}

/// The live session, as the adapter uses it.
///
/// Every method is synchronous: an implementation backed by a socket keeps its own queue and
/// answers from it, the way the collaboration branch's client does. The adapter never waits on a
/// network inside a layout pass.
pub trait Session {
    /// Join (or rejoin) the session at `role`, asking for the operations after `since`.
    ///
    /// `since = None` joins from the document; `Some(version)` is a reconnect that replays only
    /// what it missed. A relay that cannot serve the requested point refuses
    /// [`RefusalCode::Resync`]: the peer starts again from the document rather than from an
    /// incomplete tail.
    fn join(&mut self, role: Role, since: Option<u64>) -> Result<Join, Refusal>;

    /// Submit an edit built on `base`, the version the peer had applied.
    ///
    /// An `Ok` means the relay sequenced it against the persistent counter; it does not mean the
    /// peer has applied it yet — that is [`Session::receive`].
    fn submit(&mut self, edit: Edit, base: u64) -> Result<Accepted, Refusal>;

    /// The sequenced operations after `since`, oldest first.
    fn receive(&mut self, since: u64) -> Result<Vec<Delivery>, Refusal>;

    /// Publish this peer's position. Allowed without edit permission, never journalled.
    fn publish_presence(&mut self, at: Option<String>) -> Result<(), Refusal>;

    /// Who is in the document, this peer included.
    fn peers(&self) -> Vec<Peer>;

    /// Take the text the session orphaned for this peer, if any.
    fn recovered(&mut self) -> Vec<Recovered>;
}
