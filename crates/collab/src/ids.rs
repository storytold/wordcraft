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

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::wire::Version;

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
}

impl Default for NodeIds {
    fn default() -> Self {
        NodeIds::new()
    }
}

impl NodeIds {
    /// Ids start at 1 so that `0` in a target means "no node".
    pub fn new() -> NodeIds {
        NodeIds { forward: HashMap::new(), back: HashMap::new(), tombstones: HashMap::new(), next: 1 }
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
    pub fn rebase(&mut self, id: NodeId, key: NodeKey) {
        if let Some(old) = self.back.remove(&id) {
            self.forward.remove(&old);
        }
        self.tombstones.remove(&id);
        self.forward.insert(key.clone(), id);
        self.back.insert(id, key);
    }

    /// Delete an id: it is not reused for the rest of the tombstone window.
    pub fn remove(&mut self, id: NodeId, at: Version) {
        if let Some(key) = self.back.remove(&id) {
            self.forward.remove(&key);
        }
        self.tombstones.insert(id, at);
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
        for (id, mut key) in moved {
            if let Some(index) = key.path.get_mut(depth) {
                let shifted = i64::from(*index) + i64::from(delta);
                *index = u32::try_from(shifted.max(0)).unwrap_or(u32::MAX);
            }
            if let Some(old) = self.back.insert(id, key.clone()) {
                self.forward.remove(&old);
            }
            self.forward.insert(key, id);
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
}
