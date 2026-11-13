# ADR WC-0002: a peer that missed an accepted operation is closed, not left stale

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, `apps/collab-server` (peer outbox, the
  connection loop and the `/metrics` counters). Protocol-wide decisions stay in
  the collaboration bus (`ADR-0001`…`ADR-0021`); this record numbers separately
  and covers one implementation defect inside ADR-0004 ("optimistic apply, the
  server is the authority") and ADR-0006 (presence is ephemeral and droppable).
- Finding: `STATUS.md` §5.26 finding 5 — "when a peer's queue is full the
  accepted operation is dropped and the peer can stay connected, stale, if
  nothing follows" (`apps/collab-server/src/lib.rs:791-809`, CodeRabbit Major).
- Acceptance criterion affected: `TASK.md` §2 "Changes converge without loss or
  duplicates".

## Context

Every peer has a bounded outbox: `broadcast` hands each accepted operation to
the peer's `mpsc::Sender` with `try_send`, so one slow client cannot grow the
server. The bound is deliberate and stays.

What was missing is the consequence. `try_send` failing with `Full` means the
peer did **not** receive an `Applied` frame the room has already journaled. Two
things then made the peer look healthy while its replica was behind:

1. the drop was silent — no counter, no log, no state;
2. the connection stayed open. The client only learns about a gap when a later
   frame arrives (`version` jumps → resync), so with nothing following it kept a
   stale document and reported itself in sync indefinitely.

A peer's outbox fills only while that peer's connection is stuck writing to the
socket, i.e. while the peer has stopped reading. So the defect and the stalled
reader are the same failure: the transport cannot deliver, and the session does
not say so.

## Options

### A. Keep the drop, rely on the next frame

Rejected: it is the current behaviour, and "if nothing follows" is exactly the
case the finding names. Nothing in the protocol forces a frame after an edit.

### B. Per-peer stale flag, closed by the connection loop (chosen)

The outbox drop marks the peer stale. The connection loop reads the flag before
every write and on every heartbeat, and ends the connection (a `Close` frame
when the peer can still take one, a dropped socket otherwise). The client
reconnects with its `lastVersion` and either replays the journal tail or takes a
snapshot — the path that already exists and is tested
(`a_reconnecting_client_catches_up_from_the_journal`).

Cost, stated plainly: the peer loses whatever it had applied optimistically but
not confirmed — the frames it never read were the answers to its own edits. It
resumes from the snapshot. That is the same price ADR WC-0001 pays for a stale
positional edit, and for the same reason: a replica holding a document the
server does not have is worse than a resync.

### C. An unbounded outbox, or blocking `try_send`

Rejected: an unbounded outbox lets one stopped reader exhaust the server, and a
blocking send stalls the room for everyone. The bound is the reason the room
survives a stalled peer.

### D. Drop the peer from the room without ending its connection

Rejected: the connection would then answer nothing while the client believes it
is in the room, and the next operation would be refused `lease`/`gone` — a
confusing state rather than a resync.

## Decision

**B**, with one addition the flag alone cannot provide: a write bound.

A flag read by a loop that is *inside* a blocked write is never read. The loop is
blocked precisely because the peer stopped reading, so the flag and the blocked
write always occur together. Two mechanisms therefore close the gap, and either
is enough:

1. **The flag wakes the writer.** `Stale` holds the flag and a `tokio::sync::Notify`.
   A drop marks the flag and notifies; the outgoing branch races its write
   against that notification, so the write is cancelled and the socket closed as
   soon as the outbox overflows — no waiting for a timeout. The heartbeat branch
   also reads the flag, which is what closes a peer whose outbox filled while
   the loop was busy with an inbound frame.
2. **Every write is bounded by the heartbeat period.** `send` runs under
   `timeout`, and the ping and the join frames use the same bound. A peer that
   stops reading without ever filling its outbox (one frame in flight, nothing
   after it) is still closed within one period, and the client reconnects and
   resyncs.

Both paths end in `leave`: the peer is removed from the room, the others see
`left`, and the reconnecting client gets the journal tail or a snapshot.

The drop is counted: `wordcraft_collab_stale_peers_total`, once per peer, and
logged at `warn` without document content.

Presence stays droppable and never marks a peer stale: it carries no document
state, a lost frame is repaired by the next one, and marking on presence would
close a peer that has lost nothing (ADR-0006).

## Consequences

1. **A replica cannot stay silently behind.** Either it receives every accepted
   operation, or its connection ends and the join brings it to the head. The
   convergence criterion no longer depends on "a later frame will arrive".
2. **A peer that stops reading is disconnected.** A short network stall of one
   heartbeat period now costs a reconnect and a resync instead of an indefinite
   stale session. `COLLAB` operators see it in `stale_peers_total`.
3. **The outbox bound is a tuning knob.** `Config::peer_queue` (512 in
   production) and `Config::heartbeat` (`PRESENCE_TTL / 3`) are fields, not
   constants in the loop, so the tests can reach the overflow without a
   megabyte-sized socket buffer.
4. **An answer to the peer's own frame is bounded by the same write timeout, but
   not cancelled by the notification.** A peer that stops reading in the middle
   of its own operation is closed when that write times out (one heartbeat
   period), not sooner. Commit answers are left untouched on purpose: cancelling
   a commit's answer mid-flight could hide a publication the cabinet already
   stored, which the conflict path handles but should not be provoked.
5. **No protocol change.** No new frame, no new field on the wire; the client's
   existing reconnect-and-resync path does the work.

## Test evidence

- `apps/collab-server/src/tests.rs`:
  `a_peer_that_stops_reading_is_closed_and_resyncs_instead_of_staying_stale` —
  Alice pushes 64 KiB edits, Bob's socket is never read; the server marks Bob
  stale (`stale_peers_total == 1`), Alice sees `left{bob}`, Bob's socket ends,
  and Bob's reconnect with `lastVersion: 0` reports the head and replays the
  journal tail (or a snapshot), so it catches up instead of staying stale.
  Checked against the pre-fix tree on the build host: with the flag never set
  the metric assertion fails (`0 != 1`); with the write bound removed as well
  the peer is never closed and the test fails at
  `the stalled peer was closed instead of staying connected (head version 400)`
  — the finding reproduced.
- `a_full_outbox_marks_the_peer_stale_once` — the flag itself, without a socket:
  the first frame fits a one-slot outbox, the second is dropped and marks the
  peer stale, a third is the same event (counted once).
- `a_reconnecting_client_catches_up_from_the_journal` still passes: the resync
  path the close relies on is unchanged.
