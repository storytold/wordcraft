//! Version journal with `opId` deduplication (ADR-0003).
//!
//! One server is the single writer: every accepted operation gets the next
//! `version`, and a repeated `opId` returns the stored answer without running
//! the operation again. Operations are not idempotent (undo, relative moves,
//! paste), so deduplication happens **before** application.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::wire::{Applied, Op, OpId, OpKind, Reject, RejectCode, Version};

/// One journal record: who did what at which version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// JWT `sub` of the author; the client never supplies it (ADR-0008).
    pub actor: String,
    pub kind: OpKind,
    pub applied: Applied,
    /// Inverse operations that undo this entry, ready to be sent as a new
    /// operation (ADR-0010). Empty when the entry cannot be undone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inverse: Vec<Op>,
}

impl Entry {
    pub fn version(&self) -> Version {
        self.applied.version
    }
}

/// The journal of one document room.
pub struct Journal {
    head: Version,
    epoch: u64,
    entries: VecDeque<Entry>,
    dedup: HashMap<OpId, Applied>,
    order: VecDeque<OpId>,
    max_entries: usize,
    max_dedup: usize,
}

impl Journal {
    /// `max_entries` bounds the tail kept for catch-up; `max_dedup` bounds the
    /// `opId` window (ADR-0003: at least 24 h or 10 000 operations).
    pub fn new(max_entries: usize, max_dedup: usize) -> Journal {
        Journal { head: 0, epoch: 0, entries: VecDeque::new(), dedup: HashMap::new(), order: VecDeque::new(), max_entries, max_dedup }
    }

    pub fn head(&self) -> Version {
        self.head
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// First version still replayable; older bases need a snapshot.
    pub fn base(&self) -> Version {
        self.entries.front().map_or(self.head + 1, |e| e.version())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The stored answer for a repeated operation, if the window still holds it.
    pub fn cached(&self, op_id: &OpId) -> Option<&Applied> {
        self.dedup.get(op_id)
    }

    /// Validate an operation against the current head. `Err` is the refusal to
    /// send back; a cached `opId` is not an error and is checked first by the
    /// caller.
    pub fn check(&self, op: &Op) -> Result<(), Reject> {
        let reject = |code: RejectCode, message: &str| Err(Reject::new(op.op_id.clone(), code, message));
        if op.base_version > self.head {
            return reject(RejectCode::Stale, "baseVersion is ahead of the server head");
        }
        if op.base_version < self.base().saturating_sub(1) {
            return reject(RejectCode::Stale, "baseVersion was compacted out of the journal; resync from a snapshot");
        }
        if let Some(epoch) = op.epoch {
            if epoch > self.epoch {
                return reject(RejectCode::Invalid, "epoch is ahead of the server epoch");
            }
            if epoch < self.epoch {
                return reject(RejectCode::Stale, "epoch is stale; refetch the snapshot");
            }
        }
        if op.kind.is_structural() && op.base_version != self.head {
            return reject(RejectCode::Conflict, "structural operation needs baseVersion == head");
        }
        Ok(())
    }

    /// Record an accepted operation: bump `version`, `epoch` for structural,
    /// store the answer for deduplication and the entry for catch-up.
    ///
    /// The caller has already applied the operation to the model and passes the
    /// authoritative delta.
    pub fn accept(&mut self, op: &Op, actor: &str, delta: Option<serde_json::Value>, inverse: Vec<Op>) -> Applied {
        let conflict = op.base_version < self.head;
        self.head += 1;
        if op.kind.is_structural() {
            self.epoch += 1;
        }
        let applied = Applied { op_id: op.op_id.clone(), version: self.head, epoch: self.epoch, delta, conflict };
        self.remember(&applied);
        self.entries.push_back(Entry { actor: actor.to_string(), kind: op.kind, applied: applied.clone(), inverse });
        while self.entries.len() > self.max_entries {
            self.entries.pop_front();
        }
        applied
    }

    fn remember(&mut self, applied: &Applied) {
        self.dedup.insert(applied.op_id.clone(), applied.clone());
        self.order.push_back(applied.op_id.clone());
        while self.order.len() > self.max_dedup {
            if let Some(old) = self.order.pop_front() {
                self.dedup.remove(&old);
            }
        }
    }

    /// Applied answers after `version`, oldest first. `None` means the tail was
    /// compacted away and the client must take a snapshot.
    pub fn since(&self, version: Version) -> Option<Vec<Applied>> {
        if version == self.head {
            return Some(Vec::new());
        }
        if version + 1 < self.base() {
            return None;
        }
        Some(self.entries.iter().filter(|e| e.version() > version).map(|e| e.applied.clone()).collect())
    }

    /// Entries after `version` (for per-user undo and audit).
    pub fn entries_since(&self, version: Version) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.version() > version).collect()
    }

    /// Drop entries up to and including `upto` after a snapshot was written.
    pub fn compact(&mut self, upto: Version) {
        while self.entries.front().is_some_and(|e| e.version() <= upto) {
            self.entries.pop_front();
        }
    }

    /// Whether the room should write a snapshot (ADR-0005: 500 operations).
    pub fn should_snapshot(&self) -> bool {
        self.entries.len() >= 500
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{OpId, Target};

    fn op(client: &str, seq: u64, base: Version, kind: OpKind) -> Op {
        Op {
            op_id: OpId::new(client, seq),
            base_version: base,
            epoch: None,
            kind,
            target: Target::new("body", 1, 0),
            payload: serde_json::Value::Null,
        }
    }

    #[test]
    fn versions_are_monotonic_and_repeats_return_the_stored_answer() {
        let mut j = Journal::new(100, 100);
        let first = op("c1", 1, 0, OpKind::Primitive);
        j.check(&first).expect("first op is valid");
        let applied = j.accept(&first, "alice", None, Vec::new());
        assert_eq!(applied.version, 1);
        assert_eq!(j.head(), 1);

        // The same opId is deduplicated before application: the version does not move.
        assert_eq!(j.cached(&first.op_id), Some(&applied));
        assert_eq!(j.head(), 1);

        let second = op("c1", 2, 1, OpKind::Primitive);
        let applied = j.accept(&second, "alice", None, Vec::new());
        assert_eq!(applied.version, 2);
    }

    #[test]
    fn structural_needs_the_head_version() {
        let mut j = Journal::new(100, 100);
        let a = op("c1", 1, 0, OpKind::Primitive);
        j.accept(&a, "alice", None, Vec::new());

        let structural = op("c2", 1, 0, OpKind::Structural);
        let err = j.check(&structural).expect_err("stale structural is refused");
        assert_eq!(err.code, RejectCode::Conflict);
        assert!(!err.retryable);

        let fresh = op("c2", 2, 1, OpKind::Structural);
        j.check(&fresh).expect("head structural is valid");
        let applied = j.accept(&fresh, "bob", None, Vec::new());
        assert_eq!(applied.epoch, 1);
    }

    #[test]
    fn merge_lands_on_a_newer_head_with_conflict_flag() {
        let mut j = Journal::new(100, 100);
        for seq in 1..=3 {
            let o = op("c1", seq, seq - 1, OpKind::Primitive);
            j.accept(&o, "alice", None, Vec::new());
        }
        let merge = op("c2", 1, 1, OpKind::Merge);
        j.check(&merge).expect("merge may be behind");
        let applied = j.accept(&merge, "bob", None, Vec::new());
        assert!(applied.conflict);
        assert_eq!(applied.version, 4);
    }

    #[test]
    fn catch_up_returns_the_tail_and_refuses_after_compaction() {
        let mut j = Journal::new(100, 100);
        for seq in 1..=5 {
            let o = op("c1", seq, seq - 1, OpKind::Primitive);
            j.accept(&o, "alice", None, Vec::new());
        }
        let tail = j.since(2).expect("tail is available");
        assert_eq!(tail.len(), 3);
        assert_eq!(tail.first().map(|a| a.version), Some(3));

        j.compact(4);
        assert!(j.since(1).is_none(), "compacted tail needs a snapshot");
        assert_eq!(j.since(4).map(|t| t.len()), Some(1));
    }

    #[test]
    fn epoch_must_match_for_merge_operations() {
        let mut j = Journal::new(100, 100);
        let structural = op("c1", 1, 0, OpKind::Structural);
        j.accept(&structural, "alice", None, Vec::new());
        assert_eq!(j.epoch(), 1);

        let mut stale = op("c2", 1, 1, OpKind::Merge);
        stale.epoch = Some(0);
        assert_eq!(j.check(&stale).expect_err("stale epoch").code, RejectCode::Stale);

        stale.epoch = Some(1);
        j.check(&stale).expect("current epoch is valid");
    }

    #[test]
    fn dedup_window_evicts_old_ids() {
        let mut j = Journal::new(100, 2);
        for seq in 1..=3 {
            let o = op("c1", seq, seq - 1, OpKind::Primitive);
            j.accept(&o, "alice", None, Vec::new());
        }
        assert!(j.cached(&OpId::new("c1", 1)).is_none());
        assert!(j.cached(&OpId::new("c1", 3)).is_some());
    }
}
