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
        let replica = room.replica_of(&self.peer);
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

    fn replica_of(&mut self, peer: &str) -> u8 {
        if let Some(replica) = self.replicas.get(peer) {
            return *replica;
        }
        let replica = u8::try_from(self.replicas.len()).unwrap_or(u8::MAX);
        self.replicas.insert(peer.to_owned(), replica);
        replica
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
                if let (Some(block), Some(text)) = (edit.text_block(), edit.typed_text()) {
                    self.recovered.entry(peer.to_owned()).or_default().push(Recovered {
                        block,
                        text: text.to_owned(),
                        detail: "the paragraph was removed before this edit was sequenced".to_owned(),
                    });
                }
                Err(Refusal::new(RefusalCode::Gone, "the block was removed by an acknowledged edit"))
            }
            Err(EditError::Unsupported) => Err(Refusal::new(RefusalCode::Unsupported, "the adapter does not model this block or edit")),
            Err(error @ (EditError::BadOffset | EditError::LastParagraph)) => Err(Refusal::new(RefusalCode::BadRequest, error.to_string())),
        }
    }
}

fn room(room: &Arc<Mutex<Room>>) -> Result<MutexGuard<'_, Room>, Refusal> {
    lock(room)
}

fn lock(room: &Arc<Mutex<Room>>) -> Result<MutexGuard<'_, Room>, Refusal> {
    room.lock().map_err(|_| Refusal::new(RefusalCode::Resync, "the room was poisoned by a failed holder"))
}
