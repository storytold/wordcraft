//! Session-scoped node identifiers and tombstones (ADR-0002).
//!
//! The document model has no stable ids: a paragraph is addressed by an index
//! path (`doc/src/position.rs:15`), and insertion or deletion moves every later
//! path. A room therefore keeps a map `NodeKey (story, path) ↔ NodeId` for the
//! duration of the session. Clients address `(nodeId, off)`; the server
//! resolves the id back to its current path.
//!
//! The key is the whole [`Path`](wordcraft_doc::Path), so the map addresses
//! blocks of every container, not only top-level ones: a table `[b]`, its rows
//! `[b, r]`, its cells `[b, r, c]` and the paragraphs inside a cell
//! `[b, r, c, block]` each get their own id. A structural edit shifts the index
//! of the container it happened in and leaves every other index alone
//! ([`NodeIds::shift_children`]); removing a node removes its subtree.
//!
//! A deleted id is tombstoned rather than reused: an operation that arrives
//! late is refused as `Gone` instead of resurrecting a node under a new path.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::wire::{NodeDelta, NodeRef, Version};

/// The id clients put in [`crate::wire::Target`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// Where a node lives in the model right now.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeKey {
    /// `body`, `header`, `footer`, `footnote`, `comment`, `textbox`.
    pub story: String,
    /// Index path inside the story (`[block]`, `[block, row, cell, block]`, …).
    pub path: Vec<u32>,
}

impl NodeKey {
    pub fn new(story: impl Into<String>, path: Vec<u32>) -> NodeKey {
        NodeKey { story: story.into(), path }
    }
}

/// Why an id could not be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    /// The id was never issued in this session.
    Unknown,
    /// The node was deleted at this version; the operation must not land.
    Gone(Version),
}

/// The id map of one room.
#[derive(Clone, Debug)]
pub struct NodeIds {
    forward: HashMap<NodeKey, NodeId>,
    back: HashMap<NodeId, NodeKey>,
    tombstones: HashMap<NodeId, Version>,
    next: u32,
    /// Set while a room applies one operation: the ids it touches, for the
    /// delta that goes back with the answer (ADR-0002).
    delta: Option<Delta>,
}

/// What one operation did to the map, recorded while it applies.
#[derive(Clone, Debug, Default)]
struct Delta {
    changed: HashMap<NodeId, NodeKey>,
    removed: BTreeSet<NodeId>,
    /// The change set passed [`crate::wire::MAX_NODE_DELTA`]; stop recording.
    overflow: bool,
}

impl Default for NodeIds {
    fn default() -> Self {
        NodeIds::new()
    }
}

impl NodeIds {
    /// Ids start at 1 so that `0` in a target means "no node".
    pub fn new() -> NodeIds {
        NodeIds { forward: HashMap::new(), back: HashMap::new(), tombstones: HashMap::new(), next: 1, delta: None }
    }

    /// Start recording the ids an operation issues, moves or retires.
    pub fn start_delta(&mut self) {
        self.delta = Some(Delta::default());
    }

    /// Drop the recording: the delta is not going on the wire.
    pub fn cancel_delta(&mut self) {
        self.delta = None;
    }

    /// End the recording. `None` when the operation did not touch the map;
    /// `overflow` when it touched more ids than [`crate::wire::MAX_NODE_DELTA`].
    pub fn take_delta(&mut self) -> Option<NodeDelta> {
        let delta = self.delta.take()?;
        let Delta { changed, removed, overflow } = delta;
        if overflow {
            return Some(NodeDelta { overflow: true, ..NodeDelta::default() });
        }
        if changed.is_empty() && removed.is_empty() {
            return None;
        }
        // Skip an id that was retired and then put back by the same operation.
        let mut changed: Vec<(NodeId, NodeKey)> = changed.into_iter().filter(|(id, _)| !removed.contains(id)).collect();
        changed.sort_by_key(|(id, _)| *id);
        let changed = changed.into_iter().map(|(id, key)| NodeRef { id: id.0, story: key.story, path: key.path }).collect();
        Some(NodeDelta { changed, removed: removed.into_iter().map(|id| id.0).collect(), overflow: false })
    }

    fn record_changed(&mut self, id: NodeId, key: &NodeKey) {
        let Some(delta) = self.delta.as_mut() else { return };
        if delta.overflow {
            return;
        }
        delta.changed.insert(id, key.clone());
        delta.removed.remove(&id);
        if delta.changed.len() + delta.removed.len() > crate::wire::MAX_NODE_DELTA {
            // The partial map would be a lie: tell the replica to snapshot.
            *delta = Delta { overflow: true, ..Delta::default() };
        }
    }

    fn record_removed(&mut self, id: NodeId) {
        let Some(delta) = self.delta.as_mut() else { return };
        if delta.overflow {
            return;
        }
        delta.changed.remove(&id);
        if delta.removed.insert(id) && delta.changed.len() + delta.removed.len() > crate::wire::MAX_NODE_DELTA {
            *delta = Delta { overflow: true, ..Delta::default() };
        }
    }

    pub fn len(&self) -> usize {
        self.back.len()
    }

    pub fn is_empty(&self) -> bool {
        self.back.is_empty()
    }

    /// Every live id with its current path, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &NodeKey)> {
        self.back.iter().map(|(id, key)| (*id, key))
    }

    /// The id for a node, issuing one on first sight.
    /// The id a key already has, without assigning one.
    pub fn get(&self, key: &NodeKey) -> Option<NodeId> {
        self.forward.get(key).copied()
    }

    pub fn id_for(&mut self, key: &NodeKey) -> NodeId {
        if let Some(id) = self.forward.get(key) {
            return *id;
        }
        let id = NodeId(self.next);
        self.next = self.next.saturating_add(1);
        self.forward.insert(key.clone(), id);
        self.back.insert(id, key.clone());
        self.record_changed(id, key);
        id
    }

    /// The current path of an id, or why it cannot be used.
    pub fn resolve(&self, id: NodeId) -> Result<&NodeKey, Resolve> {
        if let Some(version) = self.tombstones.get(&id) {
            return Err(Resolve::Gone(*version));
        }
        self.back.get(&id).ok_or(Resolve::Unknown)
    }

    /// Move an id to a new path after a structural edit (the server calls this
    /// while applying an operation; clients never see the key).
    ///
    /// A tombstoned id is revived by this: an inverse operation that puts the
    /// node's content back restores the id with it, so a client that held the id
    /// (the caret holder of a cell a merge absorbed) addresses its own cell
    /// again.
    pub fn rebase(&mut self, id: NodeId, key: NodeKey) {
        if let Some(old) = self.back.remove(&id) {
            self.forward.remove(&old);
        }
        self.tombstones.remove(&id);
        self.forward.insert(key.clone(), id);
        self.back.insert(id, key.clone());
        self.record_changed(id, &key);
    }

    /// Replace the paths of several ids at once. Unlike calling [`NodeIds::rebase`]
    /// in a loop, this is safe for a permutation: all old forward keys are
    /// dropped before any new one is inserted, so one id's new key cannot erase
    /// another's. A fresh node map after a move or a sort uses this.
    pub fn rebase_many(&mut self, nodes: impl IntoIterator<Item = (NodeId, NodeKey)>) {
        let pairs: Vec<(NodeId, NodeKey)> = nodes.into_iter().collect();
        for (id, _) in &pairs {
            if let Some(old) = self.back.remove(id) {
                self.forward.remove(&old);
            }
        }
        for (id, key) in pairs {
            self.tombstones.remove(&id);
            self.forward.insert(key.clone(), id);
            self.back.insert(id, key.clone());
            self.next = self.next.max(id.0.saturating_add(1));
            self.record_changed(id, &key);
        }
    }

    /// Delete an id: it is not reused for the rest of the tombstone window.
    pub fn remove(&mut self, id: NodeId, at: Version) {
        if let Some(key) = self.back.remove(&id) {
            self.forward.remove(&key);
        }
        self.tombstones.insert(id, at);
        self.record_removed(id);
    }

    pub fn is_tombstoned(&self, id: NodeId) -> bool {
        self.tombstones.contains_key(&id)
    }

    /// Shift the child index of every node in `story` whose path starts with
    /// `container`, after a child at `at` was inserted (`delta` > 0) or removed
    /// (`delta` < 0) in that container.
    ///
    /// An empty container is the story's block list; `[b]` is the row list of
    /// the table at block `b`; `[b, r]` is the cell list of that row; `[b, r, c]`
    /// is the block list of that cell. Children of the shifted nodes move with
    /// them, because every path below the container keeps its trailing indexes:
    /// a row inserted above cell `(2, 1)` moves its id from `[b, 2, 1, …]` to
    /// `[b, 3, 1, …]` and the cell still names the same content.
    pub fn shift_children(&mut self, story: &str, container: &[u32], at: usize, delta: i32) {
        let depth = container.len();
        let moved: Vec<(NodeId, NodeKey)> = self
            .back
            .iter()
            .filter(|(_, key)| {
                key.story == story
                    && key.path.len() > depth
                    && key.path.starts_with(container)
                    && key.path.get(depth).is_some_and(|index| (*index as usize) >= at)
            })
            .map(|(id, key)| (*id, key.clone()))
            .collect();
        // Every old key goes first: consecutive children shift into each
        // other's keys, so removing and inserting one at a time would erase the
        // forward entry a child just wrote and leave a live node unaddressable.
        for (_, key) in &moved {
            self.forward.remove(key);
        }
        for (id, mut key) in moved {
            if let Some(index) = key.path.get_mut(depth) {
                let shifted = i64::from(*index) + i64::from(delta);
                *index = u32::try_from(shifted.max(0)).unwrap_or(u32::MAX);
            }
            self.back.insert(id, key.clone());
            self.forward.insert(key.clone(), id);
            self.record_changed(id, &key);
        }
    }

    /// Rewrite the child index of every node in `story` whose path starts with
    /// `container`, after the children of that container were reordered.
    ///
    /// A move does not insert or remove a child, so [`NodeIds::shift_children`]
    /// (which shifts everything past an index by the same amount) cannot express
    /// it: one block goes down while the run it crossed moves up. `index_of`
    /// maps an old child index to its new one; the trailing indexes below the
    /// container move with it, so a table that was moved keeps its rows, cells
    /// and the ids of the paragraphs inside it.
    pub fn permute_children(&mut self, story: &str, container: &[u32], index_of: impl Fn(u32) -> u32) {
        let depth = container.len();
        let moved: Vec<(NodeId, NodeKey, NodeKey)> = self
            .back
            .iter()
            .filter(|(_, key)| key.story == story && key.path.len() > depth && key.path.starts_with(container))
            .map(|(id, key)| {
                let mut moved = key.clone();
                if let Some(index) = moved.path.get_mut(depth) {
                    *index = index_of(*index);
                }
                (*id, key.clone(), moved)
            })
            .collect();
        // The old keys go first: in a swap, one node's new key is another's old
        // one, so removing and inserting one at a time would erase the entry the
        // earlier node just wrote.
        for (_, old, _) in &moved {
            self.forward.remove(old);
        }
        for (id, _, key) in moved {
            self.back.insert(id, key.clone());
            self.forward.insert(key.clone(), id);
            self.record_changed(id, &key);
        }
    }

    /// Delete every id at or below `path` in `story` at version `at`: a removed
    /// block, row or cell takes the nodes it contains with it. Their ids are
    /// tombstoned, so a late operation on any of them is refused as `Gone`
    /// instead of landing on whatever now sits at that path.
    pub fn remove_subtree(&mut self, story: &str, path: &[u32], at: Version) {
        let gone: Vec<NodeId> = self.back.iter().filter(|(_, key)| key.story == story && key.path.starts_with(path)).map(|(id, _)| *id).collect();
        for id in gone {
            self.remove(id, at);
        }
    }

    /// Delete every id strictly below `path` in `story` at version `at`, and
    /// keep the node at `path` itself live: a cell whose content was moved
    /// elsewhere (a merge drains a continued cell into the cell above) keeps its
    /// own id, while the ids of the blocks it used to hold must not name the
    /// blocks that now sit there.
    pub fn remove_descendants(&mut self, story: &str, path: &[u32], at: Version) {
        let gone: Vec<NodeId> = self
            .back
            .iter()
            .filter(|(_, key)| key.story == story && key.path.len() > path.len() && key.path.starts_with(path))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            self.remove(id, at);
        }
    }

    /// Shift the top-level block index of every node in `story` whose block is
    /// at or after `at`, after a block was inserted (`delta` > 0) or removed
    /// (`delta` < 0). Nested nodes move with their block; an edit inside a table
    /// cell shifts through [`NodeIds::shift_children`] with that cell's path.
    pub fn shift_top_blocks(&mut self, story: &str, at: usize, delta: i32) {
        self.shift_children(story, &[], at, delta);
    }

    /// Forget tombstones older than `before` (ADR-0003 keeps ids at least as
    /// long as the journal window, so a late operation can still be refused).
    pub fn compact(&mut self, before: Version) {
        self.tombstones.retain(|_, at| *at >= before);
    }

    pub fn tombstones(&self) -> usize {
        self.tombstones.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_key_gets_one_id() {
        let mut ids = NodeIds::new();
        let key = NodeKey::new("body", vec![3]);
        let a = ids.id_for(&key);
        let b = ids.id_for(&NodeKey::new("body", vec![3]));
        assert_eq!(a, b);
        assert_eq!(ids.resolve(a), Ok(&key));
    }

    #[test]
    fn different_stories_have_different_ids() {
        let mut ids = NodeIds::new();
        let body = ids.id_for(&NodeKey::new("body", vec![0]));
        let header = ids.id_for(&NodeKey::new("header", vec![0]));
        assert_ne!(body, header);
    }

    #[test]
    fn a_deleted_id_is_gone_and_not_reissued() {
        let mut ids = NodeIds::new();
        let id = ids.id_for(&NodeKey::new("body", vec![2]));
        ids.remove(id, 7);
        assert_eq!(ids.resolve(id), Err(Resolve::Gone(7)));

        // A new node at the same path gets a new id: ids are never reused.
        let again = ids.id_for(&NodeKey::new("body", vec![2]));
        assert_ne!(id, again);
    }

    #[test]
    fn shifting_a_run_of_children_leaves_every_forward_key_in_place() {
        // Consecutive children shift into each other's paths: the forward map
        // must still name every live child at its new path.
        let mut ids = NodeIds::new();
        let children: Vec<(NodeId, u32)> = (2..9).map(|index| (ids.id_for(&NodeKey::new("body", vec![index])), index)).collect();
        ids.shift_children("body", &[], 2, 1);
        for (id, old) in &children {
            assert_eq!(ids.resolve(*id), Ok(&NodeKey::new("body", vec![old + 1])), "child {old} moved one place");
            assert_eq!(ids.get(&NodeKey::new("body", vec![old + 1])), Some(*id), "the forward key names child {old}");
        }
        ids.shift_children("body", &[], 3, -1);
        for (id, old) in &children {
            assert_eq!(ids.get(&NodeKey::new("body", vec![*old])), Some(*id), "the forward key is back for child {old}");
        }
    }

    #[test]
    fn rebase_moves_an_id_to_its_new_path() {
        let mut ids = NodeIds::new();
        let id = ids.id_for(&NodeKey::new("body", vec![4]));
        ids.rebase(id, NodeKey::new("body", vec![5]));
        assert_eq!(ids.resolve(id), Ok(&NodeKey::new("body", vec![5])));
    }

    #[test]
    fn compaction_drops_only_old_tombstones() {
        let mut ids = NodeIds::new();
        let old = ids.id_for(&NodeKey::new("body", vec![1]));
        let recent = ids.id_for(&NodeKey::new("body", vec![2]));
        ids.remove(old, 3);
        ids.remove(recent, 9);
        ids.compact(5);
        assert!(!ids.is_tombstoned(old));
        assert!(ids.is_tombstoned(recent));
    }

    #[test]
    fn a_row_inserted_above_moves_the_cells_it_does_not_touch() {
        let mut ids = NodeIds::new();
        let table = ids.id_for(&NodeKey::new("body", vec![2]));
        let first_row = ids.id_for(&NodeKey::new("body", vec![2, 0]));
        let cell = ids.id_for(&NodeKey::new("body", vec![2, 0, 1]));
        let cell_para = ids.id_for(&NodeKey::new("body", vec![2, 0, 1, 0]));
        let above = ids.id_for(&NodeKey::new("body", vec![1]));

        ids.shift_children("body", &[2], 0, 1);

        // The table and the block above keep their place; the row, its cell and
        // the paragraph inside the cell move together.
        assert_eq!(ids.resolve(table).map(|k| k.path.clone()), Ok(vec![2]));
        assert_eq!(ids.resolve(above).map(|k| k.path.clone()), Ok(vec![1]));
        assert_eq!(ids.resolve(first_row).map(|k| k.path.clone()), Ok(vec![2, 1]));
        assert_eq!(ids.resolve(cell).map(|k| k.path.clone()), Ok(vec![2, 1, 1]));
        assert_eq!(ids.resolve(cell_para).map(|k| k.path.clone()), Ok(vec![2, 1, 1, 0]));
        // The map still answers by path: the moved id owns its new key.
        assert_eq!(ids.get(&NodeKey::new("body", vec![2, 1, 1])), Some(cell));
    }

    #[test]
    fn a_permutation_swaps_two_blocks_with_their_subtrees() {
        let mut ids = NodeIds::new();
        let first = ids.id_for(&NodeKey::new("body", vec![0]));
        let table = ids.id_for(&NodeKey::new("body", vec![1]));
        let row = ids.id_for(&NodeKey::new("body", vec![1, 0]));
        let cell_para = ids.id_for(&NodeKey::new("body", vec![1, 0, 0, 0]));
        let last = ids.id_for(&NodeKey::new("body", vec![2]));

        // One block and the table next to it swap places.
        ids.permute_children("body", &[], |index| match index {
            0 => 1,
            1 => 0,
            other => other,
        });

        assert_eq!(ids.resolve(first).map(|key| key.path.clone()), Ok(vec![1]));
        assert_eq!(ids.resolve(table).map(|key| key.path.clone()), Ok(vec![0]));
        assert_eq!(ids.resolve(row).map(|key| key.path.clone()), Ok(vec![0, 0]));
        assert_eq!(ids.resolve(cell_para).map(|key| key.path.clone()), Ok(vec![0, 0, 0, 0]));
        assert_eq!(ids.resolve(last).map(|key| key.path.clone()), Ok(vec![2]));
        // The map answers by the new path too: the table owns block 0 now.
        assert_eq!(ids.get(&NodeKey::new("body", vec![0])), Some(table));
    }

    #[test]
    fn a_row_removed_takes_its_cells_and_the_later_rows_move_up() {
        let mut ids = NodeIds::new();
        let removed_row = ids.id_for(&NodeKey::new("body", vec![0, 0]));
        let removed_cell = ids.id_for(&NodeKey::new("body", vec![0, 0, 0]));
        let removed_para = ids.id_for(&NodeKey::new("body", vec![0, 0, 0, 0]));
        let kept = ids.id_for(&NodeKey::new("body", vec![0, 1, 0, 0]));

        ids.remove_subtree("body", &[0, 0], 4);
        ids.shift_children("body", &[0], 1, -1);

        for id in [removed_row, removed_cell, removed_para] {
            assert_eq!(ids.resolve(id), Err(Resolve::Gone(4)));
        }
        assert_eq!(ids.resolve(kept).map(|k| k.path.clone()), Ok(vec![0, 0, 0, 0]));
    }

    #[test]
    fn removing_a_table_block_removes_every_node_below_it() {
        let mut ids = NodeIds::new();
        let table = ids.id_for(&NodeKey::new("body", vec![3]));
        let row = ids.id_for(&NodeKey::new("body", vec![3, 0]));
        let cell = ids.id_for(&NodeKey::new("body", vec![3, 0, 0]));
        let other = ids.id_for(&NodeKey::new("body", vec![4]));

        ids.remove_subtree("body", &[3], 2);

        assert!(ids.resolve(table).is_err());
        assert!(ids.resolve(row).is_err());
        assert!(ids.resolve(cell).is_err());
        // A block at a path that merely starts with the same digits stays.
        assert_eq!(ids.resolve(other).map(|k| k.path.clone()), Ok(vec![4]));
    }

    #[test]
    fn a_shift_inside_a_cell_leaves_the_table_and_its_other_cells_alone() {
        let mut ids = NodeIds::new();
        let table = ids.id_for(&NodeKey::new("body", vec![1]));
        let cell_a = ids.id_for(&NodeKey::new("body", vec![1, 0, 0, 0]));
        let cell_b = ids.id_for(&NodeKey::new("body", vec![1, 0, 1, 0]));

        ids.shift_children("body", &[1, 0, 0], 0, 1);

        assert_eq!(ids.resolve(table).map(|k| k.path.clone()), Ok(vec![1]));
        assert_eq!(ids.resolve(cell_a).map(|k| k.path.clone()), Ok(vec![1, 0, 0, 1]));
        assert_eq!(ids.resolve(cell_b).map(|k| k.path.clone()), Ok(vec![1, 0, 1, 0]));
    }

    #[test]
    fn a_delta_names_the_ids_an_operation_issued_moved_and_retired() {
        let mut ids = NodeIds::new();
        let old = ids.id_for(&NodeKey::new("body", vec![0]));
        let moved = ids.id_for(&NodeKey::new("body", vec![1]));
        // Work before the operation is not part of its delta.
        ids.start_delta();
        let fresh = ids.id_for(&NodeKey::new("part:5", vec![0]));
        ids.rebase(moved, NodeKey::new("body", vec![7]));
        ids.remove(old, 4);
        let delta = ids.take_delta().expect("the operation touched the map");

        assert!(!delta.overflow);
        assert_eq!(
            delta.changed.iter().find(|node| node.id == fresh.0).map(|node| (node.story.as_str(), node.path.clone())),
            Some(("part:5", vec![0]))
        );
        assert_eq!(delta.changed.iter().find(|node| node.id == moved.0).map(|node| node.path.clone()), Some(vec![7]));
        assert_eq!(delta.removed, vec![old.0]);
        // The recording is over: the next operation starts clean.
        assert!(ids.take_delta().is_none());
    }

    #[test]
    fn an_id_retired_and_restored_by_one_operation_is_not_reported_removed() {
        let mut ids = NodeIds::new();
        let id = ids.id_for(&NodeKey::new("body", vec![0]));
        ids.start_delta();
        ids.remove(id, 1);
        ids.rebase(id, NodeKey::new("body", vec![3]));
        let delta = ids.take_delta().expect("the operation changed the map");
        assert!(delta.removed.is_empty(), "{delta:?}");
        assert_eq!(delta.changed.iter().find(|node| node.id == id.0).map(|node| node.path.clone()), Some(vec![3]));
    }

    #[test]
    fn a_delta_that_passes_the_bound_reports_overflow_instead_of_a_partial_map() {
        let mut ids = NodeIds::new();
        ids.start_delta();
        for index in 0..=crate::wire::MAX_NODE_DELTA {
            ids.id_for(&NodeKey::new("body", vec![index as u32]));
        }
        let delta = ids.take_delta().expect("an overflow is still a delta");
        assert!(delta.overflow);
        assert!(delta.changed.is_empty(), "a cut-short map is never sent");
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn rebasing_an_id_above_the_counter_makes_the_next_id_free() {
        let mut ids = NodeIds::new();
        ids.rebase_many([(NodeId(40), NodeKey::new("body", vec![0]))]);
        let fresh = ids.id_for(&NodeKey::new("body", vec![1]));
        assert!(fresh.0 > 40, "a locally issued id must not collide with the server's");
    }
}
