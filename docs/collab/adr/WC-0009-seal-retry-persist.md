# ADR WC-0009: a room is sealed before it is dropped — seal, retry, persist

- Date: 2026-11-16
- Status: **draft** — proposed, not implemented. The review thread it answers
  stays open (deferred with this reason) until the rule below lands.
- Scope: WordCraft implementation, server side: `apps/collab-server/src/lib.rs`
  (`AppState::room_for`, `AppState::open_room`, `AppState::persist`,
  `AppState::build_room`, `leave`). No wire change, no document-model change.
  The storage invariants it leans on are ADR-0024's (one writer per document,
  generation pair, read-back); the restore/checkpoint economics are ADR WC-0008's.
- Finding: review `2026-11-10` on PR #5834, thread `4526700226`,
  `apps/collab-server/src/lib.rs:1380`: "Synchronize room lookup and peer
  registration with the last-peer cleanup in `leave`, so a client that has
  obtained a room cannot register into it after it is removed from
  `state.rooms`."
- Acceptance criterion affected: `TASK.md` §2 ("changes converge without loss or
  duplicates") and §7 (a document has one live room).

## Context

`room_for` does not build and publish under one lock, on purpose: parsing a
64 MiB model inside the room map would stall every other join, `/metrics` and
`persist_all` (CodeRabbit 4533747484). It builds outside the map, then takes the
map again and reuses the room a racing join published. That closes the
**builder-vs-builder** race: two joins of a never-opened document still end on
one room.

`leave` closes the **join-vs-last-leave** race with nothing:

```rust
let empty = { let guard = room.lock()…; guard.peers.is_empty() };
if empty {
    if let Err(message) = state.persist(doc, room, None) { log::warn!(…); }
    let mut rooms = state.rooms.lock()…;
    rooms.retain(|_, existing| !Arc::ptr_eq(existing, room));
}
```

A join that already holds the `Arc` from `open_room` can register its peer
between the `peers.is_empty()` check and the `retain`. The room is then removed
from the map while it holds a live peer: the snapshot written a moment earlier
does not contain that peer's state, the peer edits into a room nobody can find
again, and the next join builds a second room for the same document from the
snapshot. Two active rooms for one document is the state `TASK.md` §7 forbids,
and the edits in the orphaned one are lost on its next `leave` (it persists over
the same files from a stale version).

The reviewer's own note is right that a recheck alone is not enough: a recheck
has the same window one instruction later. The fix is a lifecycle, not a recheck.

## Proposed decision

1. **A room has a lifecycle**: `Open` → `Sealed` → dropped. The state lives in
   the same `RoomState` and is read and written under the room's own mutex — the
   mutex every journal append, every presence update and every peer registration
   already takes. There is no second lock and no lock ordering to get wrong.
2. **`leave` seals before it decides.** The last-peer test and the transition to
   `Sealed` happen in one critical section. A registration that wins the lock
   before the seal is part of the room and the room is not empty; a registration
   that arrives after it is refused (see 3).
3. **A join into a sealed room retries against the map.** Registration under the
   room mutex returns "sealed" and registers nothing; `room_for` drops the
   handle, takes the room map again, and either finds the room that replaced it
   or builds one from the snapshot the seal just wrote. The retry is bounded
   (two attempts in practice; the map re-check makes progress because the seal
   is the only writer that removes a room), and running out of attempts is a
   retryable refusal, not a panic and not an empty room.
4. **The snapshot is written once, inside the seal.** The sealed room is
   persisted before it leaves the map, under `Open`→`Sealed` as the only writer
   (ADR-0024 п.2), and the write is read back before the removal. A failed write
   leaves the room in the map, sealed and retryable: an operator's next join
   rebuilds it from the snapshot on disk and no second room is built beside it.
5. **A second room for one document is a bug, not a race to be tolerated.** The
   map is the only way to obtain a room, and a room not in the map is not
   reachable through it. Anything that removes a room without sealing it first
   is a defect by this ADR.

## Open questions

- Whether the seal should also close the room to *reads* (`snapshot`, `tail`):
  a peer that is still registered is served by its own connection, so the seal is
  proposed for new joins only.
- Whether the retry belongs in `room_for` or in the join handler: the first keeps
  the transport out of it; the second could answer the client with a retryable
  `welcome` refusal earlier. Proposed: `room_for`.
- A metric for seal retries and for "join met a sealed room" — useful to prove
  the window is closed in production, and cheap.

## Consequences

- The orphan-room window closes without holding the room map across a disk
  write: the seal is in the room mutex, the persist is where it already is.
- A join that races the last leave costs one map lookup more. That is the whole
  price.
- The rule names the invariant the tests must assert, which the current code
  cannot: "a document has at most one room, and every room that has peers is in
  the map".

## Tests to write with the implementation

- A join that registers while the last peer leaves either lands in the room
  before the seal or in the room built from the sealed snapshot — never in a room
  that is neither in the map nor persisted.
- Two joins during one seal leave one room, one snapshot and one generation
  (ADR-0024).
- A seal whose snapshot write fails keeps the room in the map and answers the
  next join from it, with no second room and no data loss.
- `persist_all` during a seal writes one pair, not two.
