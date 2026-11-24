//! An in-process relay: the backend the conformance cases run against.
//!
//! This is a test double for the contract, not a server. It keeps a room the way RFC-0020's hard
//! constraints describe it: one stable document, a sequence number that outlives a peer's socket,
//! the journal a reconnecting peer replays, an authoritative role per peer and presence that is
//! latest-wins per peer and never journalled. It also holds the document the relay is authoritative
//! for, which is what lets it refuse an edit before sequencing it — a block a structural edit
//! removed is [`RefusalCode::Gone`], and the text the edit carried goes into the author's
//! recovery list instead of resurrecting the block.
//!
//! The real backend (the collaboration branch's `apps/collab-server`) is not this: it speaks a
//! socket and a wire envelope, and it implements the same [`Session`] trait without the app
//! knowing which one is in use. This module is deliberately small and lives under the same seam, so
//! the five conformance cases exercise the adapter's ordering, role and recovery rules rather than
//! a transport.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use wordcraft_doc::Document;

use crate::edit::{Edit, EditError, apply};
use crate::ids::{BlockId, IdMap};
use crate::seam::{Accepted, Delivery, Join, Peer, Recovered, Refusal, RefusalCode, Role, Session};

/// A relay two clients can join in one process.
#[derive(Clone, Debug)]
pub struct MockRelay {
    room: Arc<Mutex<Room>>,
}

impl MockRelay {
    /// Open a room on the document the relay is authoritative for.
    pub fn new(doc: Document) -> MockRelay {
        let ids = IdMap::bind(&doc, 0);
        MockRelay { room: Arc::new(Mutex::new(Room::new(doc, ids))) }
    }

    /// Attach a peer to the room. `join` on the returned handle is a separate step, so a test can
    /// model a client that is connected but has not asked for the session yet.
    pub fn connect(&self, peer: &str) -> MockClient {
        MockClient { room: Arc::clone(&self.room), peer: peer.to_owned(), role: Role::Viewer, replica: 0, started: false }
    }

    /// The relay's copy of the document after every sequenced edit.
    pub fn document(&self) -> Option<Document> {
        room(&self.room).ok().map(|room| room.doc.clone())
    }

    /// The sequence number of the last sequenced edit.
    pub fn head(&self) -> u64 {
        room(&self.room).map(|room| room.head).unwrap_or(0)
    }

    /// How many operations the journal keeps.
    pub fn journal_len(&self) -> usize {
        room(&self.room).map(|room| room.journal.len()).unwrap_or(0)
    }

    /// The roles the room currently holds.
    pub fn roles(&self) -> BTreeMap<String, Role> {
        room(&self.room).map(|room| room.roles.clone()).unwrap_or_default()
    }
}

/// One peer's handle on a [`MockRelay`]. Cheap to drop and reconnect: the room keeps the identity.
#[derive(Debug)]
pub struct MockClient {
    room: Arc<Mutex<Room>>,
    peer: String,
    role: Role,
    replica: u8,
    started: bool,
}

impl MockClient {
    /// The peer this handle speaks for.
    pub fn peer(&self) -> &str {
        &self.peer
    }
}

impl Session for MockClient {
    fn join(&mut self, role: Role, since: Option<u64>) -> Result<Join, Refusal> {
        let mut room = lock(&self.room)?;
        let replica = room.replica_of(&self.peer)?;
        let from = since.unwrap_or(0);
        if from > room.head {
            return Err(Refusal::new(RefusalCode::Resync, format!("the relay holds {} operations; the peer asked for {}", room.head, from)));
        }
        room.roles.insert(self.peer.clone(), role);
        self.role = role;
        self.replica = replica;
        self.started = true;
        Ok(Join { version: room.head, replica, replay: room.journal_after(from), peers: room.peers() })
    }

    fn submit(&mut self, edit: Edit, base: u64) -> Result<Accepted, Refusal> {
        if !self.started {
            return Err(Refusal::new(RefusalCode::Resync, "the peer has not joined a session"));
        }
        let mut room = lock(&self.room)?;
        if !room.role_of(&self.peer).may_edit() {
            return Err(Refusal::new(RefusalCode::ReadOnly, "a viewer reads and does not write"));
        }
        room.submit(&self.peer, edit, base)
    }

    fn receive(&mut self, since: u64) -> Result<Vec<Delivery>, Refusal> {
        if since > lock(&self.room)?.head {
            return Err(Refusal::new(RefusalCode::Resync, "the peer is ahead of the relay"));
        }
        Ok(lock(&self.room)?.journal_after(since))
    }

    fn publish_presence(&mut self, at: Option<String>) -> Result<(), Refusal> {
        if !self.started {
            return Err(Refusal::new(RefusalCode::Resync, "the peer has not joined a session"));
        }
        lock(&self.room)?.presence.insert(self.peer.clone(), at);
        Ok(())
    }

    fn peers(&self) -> Vec<Peer> {
        lock(&self.room).map(|room| room.peers()).unwrap_or_default()
    }

    fn recovered(&mut self) -> Vec<Recovered> {
        lock(&self.room).map(|mut room| room.recovered.remove(&self.peer).unwrap_or_default()).unwrap_or_default()
    }
}

#[derive(Debug)]
struct Room {
    doc: Document,
    ids: IdMap,
    head: u64,
    journal: Vec<Delivery>,
    /// The version that last changed each block's text, for the stale rule.
    touched: BTreeMap<BlockId, u64>,
    roles: BTreeMap<String, Role>,
    presence: BTreeMap<String, Option<String>>,
    replicas: BTreeMap<String, u8>,
    recovered: BTreeMap<String, Vec<Recovered>>,
}

impl Room {
    fn new(doc: Document, ids: IdMap) -> Room {
        Room {
            doc,
            ids,
            head: 0,
            journal: Vec::new(),
            touched: BTreeMap::new(),
            roles: BTreeMap::new(),
            presence: BTreeMap::new(),
            replicas: BTreeMap::new(),
            recovered: BTreeMap::new(),
        }
    }

    fn replica_of(&mut self, peer: &str) -> Result<u8, Refusal> {
        if let Some(replica) = self.replicas.get(peer) {
            return Ok(*replica);
        }
        let Some(replica) = u8::try_from(self.replicas.len()).ok() else {
            return Err(Refusal::new(
                RefusalCode::NoReplica,
                format!("the session has granted all {} replica numbers", u32::from(u8::MAX).saturating_add(1)),
            ));
        };
        self.replicas.insert(peer.to_owned(), replica);
        Ok(replica)
    }

    fn role_of(&self, peer: &str) -> Role {
        self.roles.get(peer).copied().unwrap_or(Role::Viewer)
    }

    fn journal_after(&self, since: u64) -> Vec<Delivery> {
        self.journal.iter().filter(|delivery| delivery.version > since).cloned().collect()
    }

    fn peers(&self) -> Vec<Peer> {
        self.roles.iter().map(|(id, role)| Peer { id: id.clone(), role: *role, at: self.presence.get(id).cloned().flatten() }).collect()
    }

    fn submit(&mut self, peer: &str, edit: Edit, base: u64) -> Result<Accepted, Refusal> {
        if let Some(block) = edit.text_block()
            && self.touched.get(&block).is_some_and(|version| *version > base)
        {
            return Err(Refusal::new(RefusalCode::Stale, format!("the paragraph {} changed after version {base}", block)));
        }
        match apply(&mut self.doc, &mut self.ids, &edit) {
            Ok(_) => {
                self.head = self.head.saturating_add(1);
                let version = self.head;
                if let Some(block) = edit.text_block() {
                    self.touched.insert(block, version);
                }
                self.journal.push(Delivery { version, author: peer.to_owned(), edit });
                Ok(Accepted { version })
            }
            Err(EditError::Gone) => {
                if let Some((block, text)) = recoverable(&edit) {
                    self.recovered.entry(peer.to_owned()).or_default().push(Recovered {
                        block,
                        text: text.to_owned(),
                        detail: "the block this edit addressed was removed before it was sequenced".to_owned(),
                    });
                }
                Err(Refusal::new(RefusalCode::Gone, "the block was removed by an acknowledged edit"))
            }
            Err(EditError::Unsupported) => Err(Refusal::new(RefusalCode::Unsupported, "the adapter does not model this block or edit")),
            Err(error @ (EditError::BadOffset | EditError::LastParagraph)) => Err(Refusal::new(RefusalCode::BadRequest, error.to_string())),
        }
    }
}

/// The block an edit names for its author, and the text it carried, when the edit has text.
///
/// A paragraph insertion that lost its anchor names the paragraph it meant to create: the id was
/// minted before the submission and the text is its author's, so the recovery list keeps both. An
/// insertion at the top of the body can only be refused for its id, which is not a lost block.
fn recoverable(edit: &Edit) -> Option<(BlockId, &str)> {
    match edit {
        Edit::InsertText { block, text, .. } => Some((*block, text.as_str())),
        Edit::InsertParagraph { id, text, after: Some(_) } => Some((*id, text.as_str())),
        Edit::InsertParagraph { .. } | Edit::DeleteRange { .. } | Edit::RemoveParagraph { .. } => None,
    }
}

fn room(room: &Arc<Mutex<Room>>) -> Result<MutexGuard<'_, Room>, Refusal> {
    lock(room)
}

fn lock(room: &Arc<Mutex<Room>>) -> Result<MutexGuard<'_, Room>, Refusal> {
    room.lock().map_err(|_| Refusal::new(RefusalCode::Resync, "the room was poisoned by a failed holder"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::IdMap;
    use crate::seam::Role;
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Block, Paragraph};

    fn document(texts: &[&str]) -> Document {
        let mut doc = Document::new();
        doc.body.clear();
        for text in texts {
            doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
        }
        doc
    }

    #[test]
    fn a_paragraph_insertion_that_lost_its_anchor_hands_its_text_back() {
        let base = document(&["one", "two"]);
        let ids = IdMap::bind(&base, 0);
        let two = ids.id_at(1).expect("an id");
        let relay = MockRelay::new(base.clone());
        let mut alice = relay.connect("alice");
        alice.join(Role::Editor, None).expect("alice joins");
        let mut bob = relay.connect("bob");
        bob.join(Role::Editor, None).expect("bob joins");

        alice.submit(Edit::RemoveParagraph { block: two }, 0).expect("alice removes the anchor");
        let minted = BlockId(crate::ids::GENERATED_BASE);
        let refused = bob.submit(Edit::InsertParagraph { after: Some(two), id: minted, text: "typed".into() }, 0);
        assert_eq!(refused.expect_err("the anchor is gone").code, RefusalCode::Gone);

        let recovered = bob.recovered();
        assert_eq!(recovered.len(), 1, "the text the paragraph insertion carried is kept");
        assert_eq!(recovered[0].text, "typed");
        assert_eq!(recovered[0].block, minted, "the recovery names the id the peer minted for it");
    }

    #[test]
    fn a_peer_beyond_the_last_replica_number_is_refused() {
        let relay = MockRelay::new(document(&["one"]));
        for index in 0..=u8::MAX {
            let mut peer = relay.connect(&format!("peer-{index}"));
            peer.join(Role::Viewer, None).expect("there is a replica number for every peer");
        }
        let mut extra = relay.connect("peer-256");
        let refused = extra.join(Role::Viewer, None).expect_err("no replica number is left");
        assert_eq!(refused.code, RefusalCode::NoReplica);
    }
}
