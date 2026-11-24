//! Session-scoped ids for the blocks a session addresses.
//!
//! RFC-0020's constraint 3: identity spaces are granted by the relay, never self-assigned. At
//! `join` the relay hands the peer a replica number ([`IdMap::replica`]), and every id the peer
//! mints for a block it creates is drawn from its own slice of the number space. A paragraph the
//! document already had is numbered the same way on every replica — a join walks the body and
//! numbers the blocks in document order — so a text edit can address it without a round trip, and
//! the relay re-derives the same map when it applies the edit to its own copy.
//!
//! Ids are not paths. A path is an index into the body and changes when somebody inserts or
//! removes a block above it; an id stays with the block, and [`IdMap`] rebases the path side
//! after every structural edit. A block that is gone is gone: an edit that still carries its id
//! is refused ([`EditError::Gone`]) rather than landing in the block that took its place.

use std::collections::{BTreeMap, HashMap};

use wordcraft_doc::Document;

/// The first id of a peer's own space. Existing blocks are numbered below it, so the two ranges
/// cannot meet.
pub const GENERATED_BASE: u32 = 1 << 24;

/// How many ids one replica may mint. A session that creates more than a million blocks is not a
/// document anybody edits; the bound keeps every id inside its replica's slice.
pub const IDS_PER_REPLICA: u32 = 1 << 20;

/// One block of the addressed container, named for as long as it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u32);

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// The id of every block in a container, and the path each one is at now.
#[derive(Clone, Debug)]
pub struct IdMap {
    /// Body index (top-level block) → id.
    by_index: BTreeMap<u32, BlockId>,
    /// Id → body index.
    by_id: HashMap<BlockId, u32>,
    replica: u8,
    next: u32,
}

impl IdMap {
    /// Number the document's top-level blocks for a peer of replica `replica`.
    ///
    /// The body's existing blocks are numbered from `1` in document order, identically on every
    /// replica; the peer's own space starts at [`GENERATED_BASE`]. Ids the session gave this
    /// replica before a rejoin come back through [`IdMap::put`] as the journal replays, and each
    /// one moves the counter past itself.
    pub fn bind(doc: &Document, replica: u8) -> IdMap {
        let mut map = IdMap { by_index: BTreeMap::new(), by_id: HashMap::new(), replica, next: 0 };
        for index in 0..doc.body.len() {
            let id = BlockId(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
            map.by_index.insert(u32::try_from(index).unwrap_or(u32::MAX), id);
            map.by_id.insert(id, u32::try_from(index).unwrap_or(u32::MAX));
        }
        map
    }

    /// The replica number the relay granted this peer.
    pub fn replica(&self) -> u8 {
        self.replica
    }

    /// Mint an id for a block this peer creates.
    ///
    /// The counter never runs backwards, so a removed block's id is never handed out again: a
    /// retry that arrives late is refused rather than applied to a different paragraph.
    pub fn allocate(&mut self) -> BlockId {
        let within = self.next.min(IDS_PER_REPLICA.saturating_sub(1));
        self.next = self.next.saturating_add(1);
        BlockId(self.slice_base().saturating_add(within))
    }

    /// The first id of this replica's own slice.
    fn slice_base(&self) -> u32 {
        GENERATED_BASE.saturating_add(u32::from(self.replica).saturating_mul(IDS_PER_REPLICA))
    }

    /// Where an id sits inside this replica's slice, when it is one of its own.
    fn within_slice(&self, id: BlockId) -> Option<u32> {
        id.0.checked_sub(self.slice_base()).filter(|within| *within < IDS_PER_REPLICA)
    }

    /// The index the id is at, or `None` when the block is gone.
    pub fn index(&self, id: BlockId) -> Option<usize> {
        self.by_id.get(&id).map(|i| *i as usize)
    }

    /// The id at an index, when the index holds an addressed block.
    pub fn id_at(&self, index: usize) -> Option<BlockId> {
        self.by_index.get(&u32::try_from(index).ok()?).copied()
    }

    /// How many blocks are addressed.
    pub fn len(&self) -> usize {
        self.by_index.len()
    }

    /// Whether no block is addressed.
    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }

    /// Put a newly created block at `index` and record its id.
    ///
    /// An id from this replica's own slice moves the counter past it, wherever the block came
    /// from: the peer's own insertion, a journal replayed after a resync, or the relay's copy of
    /// the document. A session numbers its blocks from the same space after a rejoin, so the
    /// counter has to remember what it gave out before the replica went away — otherwise the next
    /// insertion would mint an id a delayed edit still names.
    pub fn put(&mut self, index: usize, id: BlockId) {
        let Ok(index) = u32::try_from(index) else { return };
        if let Some(old) = self.by_index.insert(index, id) {
            self.by_id.remove(&old);
        }
        self.by_id.insert(id, index);
        if let Some(within) = self.within_slice(id) {
            self.next = self.next.max(within.saturating_add(1));
        }
    }

    /// Note that a block was inserted at `index`: every block at or after it moved down one.
    pub fn shifted(&mut self, index: usize) {
        let Ok(at) = u32::try_from(index) else { return };
        // From the end, so a moved id never lands on one that has not moved yet.
        let tail: Vec<(u32, BlockId)> = self.by_index.range(at..).map(|(i, id)| (*i, *id)).collect::<Vec<_>>();
        for (i, id) in tail.into_iter().rev() {
            self.by_index.remove(&i);
            let moved = i.saturating_add(1);
            self.by_index.insert(moved, id);
            self.by_id.insert(id, moved);
        }
    }

    /// Forget the block at `index` and move every block after it up one.
    pub fn removed(&mut self, index: usize) {
        let Ok(at) = u32::try_from(index) else { return };
        if let Some(id) = self.by_index.remove(&at) {
            self.by_id.remove(&id);
        }
        let tail: Vec<(u32, BlockId)> = self.by_index.range(at..).map(|(i, id)| (*i, *id)).collect::<Vec<_>>();
        for (i, id) in tail {
            self.by_index.remove(&i);
            let moved = i.saturating_sub(1);
            self.by_index.insert(moved, id);
            self.by_id.insert(id, moved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Block, Paragraph};

    fn three_paragraphs() -> Document {
        let mut doc = Document::new();
        doc.body.clear();
        for text in ["one", "two", "three"] {
            doc.body.push(std::sync::Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
        }
        doc
    }

    #[test]
    fn every_replica_numbers_the_existing_blocks_alike() {
        let doc = three_paragraphs();
        let a = IdMap::bind(&doc, 1);
        let b = IdMap::bind(&doc, 7);
        assert_eq!(a.id_at(0), b.id_at(0));
        assert_eq!(a.id_at(2), Some(BlockId(3)));
        assert_eq!(a.replica(), 1);
    }

    #[test]
    fn two_replicas_mint_ids_from_their_own_space() {
        let doc = three_paragraphs();
        let mut a = IdMap::bind(&doc, 1);
        let mut b = IdMap::bind(&doc, 2);
        let first = a.allocate();
        let second = b.allocate();
        assert_ne!(first, second);
        assert!(first.0 >= GENERATED_BASE, "a minted id is above every id an existing block got");
        assert!(second.0 >= GENERATED_BASE);
        assert_ne!(a.allocate(), first);
    }

    #[test]
    fn a_replayed_block_moves_the_counter_past_its_id() {
        let doc = three_paragraphs();
        let mut ids = IdMap::bind(&doc, 1);
        let first = ids.allocate();
        ids.put(3, first);

        // The replica rejoins: the map is numbered again and the journal replays the insertion
        // with the id it was given before the peer went away.
        let mut again = IdMap::bind(&doc, 1);
        again.put(3, first);
        let next = again.allocate();
        assert_ne!(next, first, "a rejoin does not hand out an id the session already gave");
        assert!(next.0 > first.0, "the counter resumes past the ids the replay brought back");
    }

    #[test]
    fn a_shifted_block_keeps_its_id_and_moves_its_path() {
        let doc = three_paragraphs();
        let mut ids = IdMap::bind(&doc, 1);
        let two = ids.id_at(1).expect("an id");
        let fresh = ids.allocate();
        ids.shifted(1);
        ids.put(1, fresh);
        assert_eq!(ids.index(two), Some(2));
        assert_eq!(ids.index(fresh), Some(1));
        ids.removed(0);
        assert_eq!(ids.index(fresh), Some(0));
        assert_eq!(ids.index(two), Some(1));
        assert_eq!(ids.index(BlockId(1)), None);
    }
}
