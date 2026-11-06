//! Session-scoped node identifiers and tombstones (ADR-0002).
//!
//! The document model has no stable ids: a paragraph is addressed by an index
//! path (`doc/src/position.rs:15`), and insertion or deletion moves every later
//! path. A room therefore keeps a map `NodeKey (story, path) ↔ NodeId` for the
//! duration of the session. Clients address `(nodeId, off)`; the server
//! resolves the id back to its current path.
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

    /// Shift the top-level block index of every node in `story` whose block is
    /// at or after `at`, after a block was inserted (`delta` > 0) or removed
    /// (`delta` < 0). Deeper paths (table cells) are not shifted: v1 addresses
    /// top-level blocks only.
    pub fn shift_top_blocks(&mut self, story: &str, at: usize, delta: i32) {
        let moved: Vec<(NodeId, NodeKey)> = self
            .back
            .iter()
            .filter(|(_, key)| key.story == story && key.path.first().is_some_and(|first| (*first as usize) >= at))
            .map(|(id, key)| (*id, key.clone()))
            .collect();
        for (id, mut key) in moved {
            if let Some(first) = key.path.first_mut() {
                let shifted = i64::from(*first) + i64::from(delta);
                *first = u32::try_from(shifted.max(0)).unwrap_or(u32::MAX);
            }
            if let Some(old) = self.back.insert(id, key.clone()) {
                self.forward.remove(&old);
            }
            self.forward.insert(key, id);
        }
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
}
