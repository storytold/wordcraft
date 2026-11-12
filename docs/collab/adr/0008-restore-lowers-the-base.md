# ADR WC-0008: a restored room lowers the client's base and takes the queue it replaced

- Date: 2026-11-12
- Status: accepted
- Scope: WordCraft implementation, client side: `crates/collab-client`
  (`Client::on` for `Welcome` and `Snapshot`, `Client::adopt_head`, the
  `Event::Welcome` payload). No wire change, no server change, no document
  model change. Protocol-wide decisions stay in the collaboration bus
  (`ADR-0001`…`ADR-0021`); this record numbers separately.
- Finding: review `5853542478`, body only, no inline id —
  `crates/collab-client/src/lib.rs:297` at `50c68e3a`: "snapshot reconciliation
  keeps `max(version)` when the server restores an older snapshot"
  (`STATUS.md` §4, §5.35).
- Acceptance criterion affected: `TASK.md` §2 "Changes converge without loss or
  duplicates" and "Reconnect and resync".

## Context

The client's `version` is the base every operation it builds carries
(`Op::base_version`), and the room refuses an operation whose base is above its
own head before it looks at anything else:

```rust
// crates/collab/src/journal.rs
if op.base_version > self.head {
    return reject(RejectCode::Stale, "baseVersion is ahead of the server head");
}
```

Both frames that carry the room's version assigned it with `max`:

```rust
ServerMsg::Welcome { version, .. } => self.version = self.version.max(version),
ServerMsg::Snapshot { version, .. } if last => self.version = self.version.max(version),
```

So a room that came back **lower** than the replica — an operator restoring an
older `model.json` + `meta`, which ADR WC-0003 and the store allow, and which is
the documented way out of a bad snapshot — left the client's base above the
head for the rest of the session. Nothing in the protocol raises it back:
`Applied` versions are the room's, so every operation the client built was
refused `Stale`, and each refusal asked for another snapshot at the same
version. The session was live, the document was visible, and no edit could land.

A second fact makes the repair a decision rather than an assignment: the client
holds a queue of operations the room has not answered (`pending`). Those
operations were **applied optimistically** to the replica's document — that is
why the author's own answer is skipped as a duplicate (`SyncState::apply`:
`applied.op_id.client == self.client_id && applied.history.is_none()` → nodes
only). The snapshot that follows a restore replaces that document, so the
optimistic copy of every queued edit is gone from the replica while the
operation is still in the queue.

## Decision

Reconcile the base and the queue in one place, `Client::adopt_head(head)`, and
use it for both frames.

1. **The base is the head, not a maximum.** `adopt_head` assigns `head`
   unconditionally, so a snapshot — which *is* the room's document at that
   version — always leaves the replica on the version of the document it shows.
   The ordinary case only moves it forward.
2. **A head below the base takes the queue with it.** Every pending operation is
   removed and returned to the caller, and an in-flight undo/redo entry is
   removed with it. The rule is not "drop the ones above the head": an operation
   with a base at or below the head was still computed on a document that held
   the edits above the head, so its offsets or its address may describe text the
   restored document does not have. RFC §4.5.2 already said the client
   "replaces the model and drops the unaccepted queue" on a snapshot; this is
   that sentence, applied where it is decidable.
3. **None of them is dropped silently.** `Event::Welcome` carries
   `dropped: Vec<OpId>` — the operations the restore took — so the app can name
   them. An empty vector is the ordinary handshake.
4. **A rollback with no snapshot cannot be joined.** The server always answers a
   `last_version` above its head with a snapshot (`JoinPlan::snapshot`), so this
   branch is defensive: if the frame says `resync: false` while the head is
   below the base, the client does not report a welcome it cannot honour — it
   returns `Event::Resync` and asks for the document.
5. **Identity survives.** `seq` keeps counting and the `confirmed` window keeps
   every answer at or below the head, so `(client_id, seq)` stays unique for the
   session and an operation the restored room still remembers is still
   deduplicated by its `opId`. Confirmations above the head are pruned — the
   `dedup.json` that ships with a snapshot is cut to the snapshot's version, so
   the room cannot answer them — and the history steps they named leave the
   undo/redo stacks with them.

**Why not re-plan the queue onto the new head.** Rewriting `base_version` would
pass `journal.check` and then land an offset-based edit on text it was not
computed on (`ADR WC-0001` refuses exactly that when the base is merely
behind). And a replayed operation is invisible to its own author: the author
skips its own delta, because the optimistic copy used to be in the document the
snapshot just replaced. Re-planning is only sound for the class that applies on
any base (`merge`, independent property edits) and only if the app re-applies
the effect after the snapshot; that is a different change, with its own test
matrix, and it is not needed to close this defect.

## Consequences

1. **A restored room is usable again.** The replica's base equals the head, its
   next operation is accepted, and no `Stale` is produced by the restore itself.
2. **The price is named and visible.** Edits that were in flight when the room
   went back are lost — the room never had them, or had them in a version the
   restore discarded — and the welcome names each `opId`. The alternative
   (silently keeping a queue that cannot be applied) is what the defect was.
3. **A restore also costs the undo history above the head.** Steps confirmed in
   the discarded versions leave the stacks with their confirmations. Steps at or
   below the head stay; the room's own undo stacks are in memory and do not
   survive a restart, so an undo of such a step is refused `undoConflict` — the
   refusal is explicit and ADR-0010 already defines it.
4. **The UI wiring is one line per host and is not in this change.** The
   desktop session and the web session destructure `Event::Welcome` with `..`,
   so the field compiles untouched; showing "the document was restored; N edits
   were not replayed" is left to the panel pass that owns the wording
   (`STATUS.md` §5.41, "what is left"). Until then the information exists at the
   client boundary and is not rendered.
5. **Not covered: a snapshot below the base that no welcome announced.** The
   snapshot path reconciles the queue the same way, but its event has no field
   for the list, so in that (unreachable, per the join plan) case the drop would
   be silent. It is named here rather than implied.
6. **Not covered: the app's own version copy.** `CollabSession`/the web session
   keep `SyncState::version` with the same `max` on the welcome path; the
   snapshot that follows sets it to the room's version. Between the two frames
   the panel can show the old number for a moment. Nothing builds an operation
   from that copy (the client owns `base_version`), so no refusal comes from it.

## Test evidence

`crates/collab-client/src/tests.rs` (26 tests in the package, all green):

- `a_welcome_below_the_base_lowers_it_and_reports_the_queue_it_drops` — welcome
  `7`, two queued operations, welcome `3`: base `3`, queue empty,
  `dropped == [seq 1, seq 2]`, state `Resyncing`.
- `a_restored_room_does_not_refuse_the_next_operation_stale` — a real
  `Room::restore` at version `0` against a client that had confirmed version
  `1`: the queued operation with base `1` is refused `Stale` by the room (the
  defect, asserted), the client has nothing to resend, and the operation it
  builds on the lowered base is accepted as version `1`.
- `two_clients_converge_after_the_room_is_restored` — two replicas at version
  `3`, a room restored from the snapshot at version `1`: both lower to `1`,
  exchange one answer each without a gap, and end on the room's head with empty
  queues.
- `a_restore_keeps_the_dedup_window_below_the_head_and_the_id_counter` — an
  answer at version `1` stays deduplicated, the one at version `2` does not, its
  history step leaves the stack, and the next id is `seq 3` (never reused).
- `a_snapshot_below_the_base_lowers_it_and_clears_the_queue` — the snapshot path
  lowers the base to `4` and clears a queue built at `9`.
- `a_rollback_without_a_snapshot_asks_for_one` — welcome `5` → welcome `2` with
  `resync: false` returns `Event::Resync`, not a welcome.

Checked on the build host that the six fail without the fix: with the two
assignments put back to `self.version.max(version)` (the pre-fix code, the
`dropped` list forced empty), all six fail and the other twenty pass. The patch
was reverted afterwards; the tree is clean.
