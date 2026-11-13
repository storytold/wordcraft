# ADR WC-0005: a `client_id` belongs to the connection that registered it

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, `apps/collab-server` (peer registration, the
  `client_id` claim in the auth frame, `leave`, and one `/metrics` counter).
  Protocol-wide decisions stay in the collaboration bus (`ADR-0001`…`ADR-0021`);
  this record numbers separately and covers one implementation defect inside
  ADR-0006 (presence is ephemeral) and ADR-0008 (the token decides who a
  connection is).
- Finding: `STATUS.md` §5.26 finding 6 — "any authenticated participant can
  register another's `client_id`; a stale connection can also remove the
  replacement's entry" (`apps/collab-server/src/lib.rs:448-449` at `af9a43a2`;
  CodeRabbit Major).
- Acceptance criterion affected: `TASK.md` §2 "Changes converge without loss or
  duplicates" and §5 "the participant list shows who is in the document".

## Context

The room keys its peers by the `client_id` the client puts in its auth frame:

- `register` did `peers.insert(client_id, peer)`, so a second connection with
  the same id **replaced** the first one's entry without telling it;
- `leave(client_id)` did `peers.remove(client_id)`, so the first connection's
  `leave` removed whatever connection held the id at that moment.

Two consequences, and the second is not hypothetical:

1. **A participant can take another's id over.** Any authenticated participant
   who sends `client_id: "alice"` displaces Alice's entry. Inside one cabinet
   session (`sid`, ADR-0008) the lease does not stop it; as `owner` the lease
   does not stop it either. Alice keeps a socket that receives nothing from the
   room while the list shows her under the intruder's role.
2. **A reconnect races its own predecessor.** A peer is closed (stale outbox,
   ADR WC-0002) or its socket dies; the client reconnects with the same
   `client_id` and `lastVersion` — that is the reconnect path ADR WC-0002 and
   §5.31 depend on. The server can accept the new connection *before* the old
   task runs `leave`, because the two sockets are served by two tasks. Then
   `register(new)` → `leave(old)` removes the new entry: the reconnected client
   is in the room for the room's operations but is not a participant, receives
   no broadcasts and is not counted for the lease.

The id itself must stay the client's: `opId = (clientId, seq)` is what
deduplicates a retry across a reconnect (ADR-0003), and the unconfirmed queue is
keyed by it. The server cannot hand a new id out mid-session without breaking
retries.

## Options

### A. Refuse every second registration of a live `client_id`

Rejected as the whole rule. It is right for the intruder and wrong for case 2:
the reconnecting client is refused while the server still holds the socket it
could not notice is gone. The only code a join refusal has is `invalid` (not
retryable), so the window would stop instead of reconnecting — a regression of
the path ADR WC-0002 exists to provide — while a retryable `lease` would turn
every reconnect into a wait for the heartbeat.

### B. Connection identity, takeover only inside the same token identity (chosen)

`Peer` carries a server-assigned `conn` id plus the `sub` and `sid` the
connection authenticated as. A registration of a taken id is:

- **refused** (`invalid`, not retryable, counted) when `sub` or `sid` differs —
  the claim is not this connection's;
- **a takeover** when both match — this client reconnecting. The old peer is
  retired (its loop gets a signal and ends the socket, so two sockets never
  share one `(clientId, seq)` space) and the map holds the new peer.

`leave` removes a peer only when the stored `conn` equals the caller's. The old
connection's `leave` therefore cannot remove the new one, in either order.

### C. The server assigns the `client_id`

Rejected. It is a wire change in a protocol whose ids are the retry key
(ADR-0003): the client's queue and pending `opId`s would have to be rewritten on
every join, and a client that reconnects would have to be answered with the id
it used before — which is the same problem, one frame later. Ownership by the
token costs no frame.

### D. Compare the peer's outbox channel instead of a counter

Rejected as the mechanism, worth naming: `mpsc::Sender::same_channel` would
answer "is this the peer I registered" without a new field. But `leave` would
then depend on the channel value, which is cloned into every broadcast clone of
`Peer`, and the refusal rule still needs `sub`/`sid` on the peer. An explicit
number says what the invariant is and shows up in a test assertion.

## Decision

**B**, in four rules:

1. **Ownership is derived from the token, never from the frame.** The
   auth frame picks the `client_id`; the JWT says who is claiming it
   (`sub`, `sid`). A peer entry records both.
2. **Takeover requires equality of both.** Same `sub` and same `sid` means the
   same client reconnecting: the entry is replaced and the old connection is
   retired with a `close` frame. Different `sub` or `sid` means a claim on
   someone else's id: refused `invalid`, not retryable, logged without document
   content, counted as `wordcraft_collab_client_id_conflicts_total`.
3. **Removal is by connection, not by id.** `leave(client_id, conn)` removes the
   entry only when `conn` matches. A retired connection's `leave` is a no-op: no
   `left` frame is broadcast, and the room is not treated as empty, so it is not
   snapshotted-and-dropped under a live replacement.
4. **No wire change.** No new frame and no new field; the client keeps its id,
   its reconnect and its retry dedup (§5.31 is unaffected).

The `Retire` signal is the type ADR WC-0002 introduced as `Stale`: a flag, a
`Notify`, and a wait. The connection loop now waits on two of them — the outbox
overflow and the takeover — and ends the socket on either.

## Consequences

1. **Presence cannot be displaced.** The participant list is keyed by an id only
   its owner can re-register; an intruder is refused before it reaches the map,
   and the victim's socket keeps working.
2. **The reconnect race is safe.** The reconnect that used to remove itself now
   leaves the room with the new connection as its only entry, whichever of
   `register`/`leave` runs first.
3. **A zombie connection is closed promptly.** After a takeover the old socket
   receives `close` instead of staying open and issuing operations under an id
   that now belongs to the new connection.
4. **The rule assumes `sid` is stable for the life of a `client_id`.** That is
   how the cabinet issues tokens (ADR-0008) and how the desktop transport
   reconnects (it keeps its token and its id). A client that renews its token
   *and* changes `sid` while keeping the id would be refused `invalid`; the web
   bridge avoids this by deriving a fresh `client_id` per connection
   (`session_client_id`), and reopening the document is the recovery. Recorded
   here so the assumption is visible, not implied.
5. **An operator learns about claims.** `client_id_conflicts_total` growing means
   a client is using an id that is not its own — a bug in an integration or an
   attempt; the log names the document only.
6. **Costs nothing per operation.** Registration is once per connection; the
   room's hot path (broadcast, presence, operations) is unchanged.

## Test evidence

- `apps/collab-server/src/tests.rs`:
  `another_participant_cannot_register_a_taken_client_id` — Alice joins; a
  second connection authenticates as Bob and registers `alice`: it is answered
  `rejected{invalid, retryable: false}`, its socket ends, the room still holds
  one peer (`sub == "alice"`), Alice's next operation is applied, and
  `client_id_conflicts_total == 1`.
- `a_reconnecting_client_takes_its_client_id_over_and_the_old_leave_keeps_the_new_peer`
  — Carol observes; Alice joins, then Alice reconnects with the same id (same
  `sub`, same `sid`). The old socket receives `close`, Carol receives no `left`
  (the old connection's `leave` is a no-op), the room still lists two peers, and
  the new connection's operation is applied and broadcast to Carol.
- `a_taken_client_id_is_only_removed_by_the_connection_that_registered_it` — the
  same three orderings without sockets, calling `register`/`leave` directly: a
  foreign claim is `Err(invalid)`; the same identity takes over; `leave` with the
  old `conn` keeps the replacement and broadcasts nothing; `leave` with the new
  `conn` removes it.
- Checked against the pre-fix tree on the build host: the refusal test fails at
  its first `match` (the room accepts the claim, `Welcome` arrives), and the
  takeover test fails on the `left` Carol receives and on the room's peer count;
  the direct test fails at `leave` removing the replacement. The patch was
  reverted afterwards; the tree is clean.
