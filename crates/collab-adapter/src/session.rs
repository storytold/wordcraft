//! The adapter's client half: one document, one session seam, one sequence.
//!
//! A [`DocSession`] owns a replica. It joins a session, applies what the relay sequenced in the
//! relay's order, submits the person's edits built on the version it has applied, and keeps the
//! block ids in step with the document (`crate::ids`). The relay's copy and this one converge
//! because both apply the same edits in the same order through the same document primitives — not
//! because the client guesses what another client meant.
//!
//! Two rules the seam states, made concrete here:
//!
//! - **A viewer is refused before the submission leaves the process** and the relay refuses it
//!   again. The adapter is not the only gate (RFC-0020, roles).
//! - **A reconnect replays the journal, it does not rebuild from nothing.** The peer asks for the
//!   operations after the version it applied; a relay that cannot serve that point answers
//!   [`RefusalCode::Resync`], and only then does the replica rebuild from the document and replay
//!   the whole journal.
//!
//! A stale submission is not an error the person sees: [`DocSession::submit`] hands the refusal
//! back. The offsets the edit carried were built on the paragraph as this replica had it, so the
//! caller rebuilds the edit against the paragraph's text after syncing — resubmitting the same
//! bytes can address characters somebody else inserted.

use wordcraft_doc::Document;

use crate::edit::{Edit, apply};
use crate::ids::{BlockId, IdMap};
use crate::seam::{Delivery, Peer, Recovered, Refusal, RefusalCode, Role, Session};

/// A joined replica of the document.
pub struct DocSession {
    /// The document as this replica now has it.
    pub doc: Document,
    /// The block ids of [`DocSession::doc`].
    pub ids: IdMap,
    /// The document the relay was opened on, for a resync that has to start again from it.
    base: Document,
    peer: String,
    role: Role,
    version: u64,
    transport: Box<dyn Session>,
}

impl DocSession {
    /// Join the session on `transport` with this replica's copy of `doc`.
    ///
    /// The replica number comes from the relay, and every id this replica mints for a block it
    /// creates is drawn from that space. The document is not modified until a sequenced edit says
    /// so.
    pub fn attach(peer: &str, role: Role, doc: Document, mut transport: Box<dyn Session>) -> Result<DocSession, Refusal> {
        let join = transport.join(role, None)?;
        let ids = IdMap::bind(&doc, join.replica);
        let base = doc.clone();
        let mut session = DocSession { doc, ids, base, peer: peer.to_owned(), role, version: 0, transport };
        session.apply(&join.replay)?;
        session.ensure_current(join.version)?;
        Ok(session)
    }

    /// What this peer is called in the session.
    pub fn peer(&self) -> &str {
        &self.peer
    }

    /// The role the relay holds for this peer.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The sequence number this replica has applied.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The body's text, the plain view two replicas are compared by.
    pub fn text(&self) -> String {
        self.doc.plain_text(wordcraft_doc::StoryRef::Body)
    }

    /// Mint an id of a paragraph this replica is about to insert.
    ///
    /// Fails only when this replica has minted every id of its slice; the caller then has no
    /// identity left to create a block with and is told so instead of sharing one.
    pub fn next_block_id(&mut self) -> Result<BlockId, Refusal> {
        self.ids.allocate()
    }

    /// Submit an edit built on the version this replica has applied.
    ///
    /// On `Ok` the edit is sequenced and already applied here; other replicas see it on their next
    /// sync.
    ///
    /// A [`RefusalCode::Stale`] refusal means nothing was sequenced. The caller syncs and rebuilds
    /// the edit against the paragraph as it now reads — the offsets of the refused edit describe a
    /// text that has since moved — then submits the rebuilt one.
    ///
    /// A [`RefusalCode::Resync`] refusal is the other case: the relay answered `Ok` and only the
    /// [`DocSession::sync`] that follows failed. The edit *is* sequenced, this replica is behind,
    /// and the caller brings it up to date by syncing or reconnecting. Submitting the same edit
    /// again would be a second, non-idempotent edit, so a `Resync` is never retried as a
    /// submission.
    pub fn submit(&mut self, edit: Edit) -> Result<u64, Refusal> {
        if !self.role.may_edit() {
            return Err(Refusal::new(RefusalCode::ReadOnly, "a viewer reads and does not write"));
        }
        self.transport.submit(edit, self.version)?;
        if let Err(refusal) = self.sync() {
            return Err(Refusal::new(
                RefusalCode::Resync,
                format!("the edit was sequenced but this replica could not apply it ({refusal}); sync or reconnect, do not resubmit"),
            ));
        }
        Ok(self.version)
    }

    /// Insert a paragraph after `after` (or at the top), minting its id first.
    pub fn insert_paragraph(&mut self, after: Option<BlockId>, text: &str) -> Result<BlockId, Refusal> {
        let id = self.next_block_id()?;
        self.submit(Edit::InsertParagraph { after, id, text: text.to_owned() })?;
        Ok(id)
    }

    /// Apply every operation the relay sequenced after this replica's version.
    pub fn sync(&mut self) -> Result<usize, Refusal> {
        let deliveries = self.transport.receive(self.version)?;
        let count = deliveries.len();
        self.apply(&deliveries)?;
        Ok(count)
    }

    /// Rejoin through a new transport, replaying only what this replica missed.
    pub fn reconnect(&mut self, mut transport: Box<dyn Session>) -> Result<(), Refusal> {
        match transport.join(self.role, Some(self.version)) {
            Ok(join) => {
                self.apply(&join.replay)?;
                self.ensure_current(join.version)?;
                self.transport = transport;
                Ok(())
            }
            Err(refusal) if refusal.code == RefusalCode::Resync => {
                // The journal cannot serve the point this replica applied: start again from the
                // document and replay the whole session.
                let join = transport.join(self.role, None)?;
                self.doc = self.base.clone();
                self.ids = IdMap::bind(&self.doc, join.replica);
                self.version = 0;
                self.apply(&join.replay)?;
                self.ensure_current(join.version)?;
                self.transport = transport;
                Ok(())
            }
            Err(refusal) => Err(refusal),
        }
    }

    /// Who is in the document now.
    pub fn peers(&self) -> Vec<Peer> {
        self.transport.peers()
    }

    /// Tell the session where this peer is. Allowed to a viewer, never journalled.
    pub fn set_caret(&mut self, at: Option<&str>) -> Result<(), Refusal> {
        self.transport.publish_presence(at.map(str::to_owned))
    }

    /// Text an acknowledged structural edit orphaned, handed back to this peer.
    pub fn recovered(&mut self) -> Vec<Recovered> {
        self.transport.recovered()
    }

    fn apply(&mut self, deliveries: &[Delivery]) -> Result<(), Refusal> {
        for delivery in deliveries {
            if delivery.version <= self.version {
                continue;
            }
            if delivery.version != self.version.saturating_add(1) {
                return Err(Refusal::new(
                    RefusalCode::Resync,
                    format!("the journal skipped version {} (this replica applied {})", delivery.version, self.version),
                ));
            }
            apply(&mut self.doc, &mut self.ids, &delivery.edit)
                .map_err(|error| Refusal::new(RefusalCode::Resync, format!("version {} does not apply here: {error}", delivery.version)))?;
            self.version = delivery.version;
        }
        Ok(())
    }

    fn ensure_current(&self, version: u64) -> Result<(), Refusal> {
        if version == self.version {
            return Ok(());
        }
        Err(Refusal::new(RefusalCode::Resync, format!("the relay is at version {version} and this replica applied {}", self.version)))
    }
}

impl std::fmt::Debug for DocSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocSession")
            .field("peer", &self.peer)
            .field("role", &self.role)
            .field("version", &self.version)
            .field("blocks", &self.doc.body.len())
            .finish()
    }
}
