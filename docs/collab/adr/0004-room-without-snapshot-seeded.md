# ADR WC-0004: a room is seeded by the first writer, never started blank

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, `apps/collab-server` (the room table, the
  handshake and `/metrics`) plus the first frames of the client handshake
  (`crates/collab`, `crates/collab-client`, `crates/collab-ws`,
  `crates/collab-app`, `crates/collab-web`). Protocol-wide decisions stay in the
  collaboration bus (`ADR-0001`…`ADR-0021`); this record numbers separately and
  covers one implementation gap inside ADR-0005 (the server snapshots the room)
  and ADR-0011 (the snapshot is the model plus the source `.docx`).
- Finding: `STATUS.md` §5.26 finding 3 — "a room without a snapshot starts
  blank; its snapshot replaces the open document and loses the text"
  (`apps/collab-server/src/lib.rs:240-270`, the `Ok(None)` branch of `Store::load`).
- Acceptance criterion affected: `TASK.md` §3 "nothing the server cannot model is
  lost" and §2's "without loss": losing the text a participant is looking at is
  the largest loss a room can cause.

## Context

`AppState::room_for` created a room on the first join. When the store had no
snapshot for the document (`Ok(None)`, the branch §5.30 made explicit), the room
was built from `Document::new()`. `Room::join` answers a client that has no
`lastVersion` with a snapshot — so the joining client received the empty
document and `Session::set_document` replaced the document it had open.

The loss is silent and total for that client: the text it had open, including
edits it had not committed, is gone from its window; the window then reports
`InSync` and the next commit writes an empty `.docx` over the cabinet's copy.
Nothing on the wire said "this room has no document yet": the protocol had no
seeding step, and `last_version: None` meant both "I have nothing" and "I have
the document, tell me what the room holds".

The two cases the server conflated:

- **a new document** — the file was created in the app a moment ago; the blank
  room is right, and the client's own copy is blank too;
- **an existing document** — the cabinet holds a file and the client has it
  open; a blank room is a wrong answer to a question nobody asked.

Only the client knows the difference, because only the client has the document.
The server cannot read the cabinet (the callback is a commit `POST`, not a
download, and a desktop session has no cabinet at all).

## Options

### A. Keep the blank room, make the client keep its document

The client would ignore a snapshot that arrives for a room at version 0 and keep
its copy. Rejected: the room stays blank on the server. The client's first edit
is computed against a document the server does not have, every delta comes back
wrong, and the commit still writes an empty page. Hiding the divergence in the
client is the failure mode §5.26 finding 1 and finding 5 are about.

### B. Refuse every join to a room with no snapshot

Rejected as a whole: collaboration on an existing document would never start
after a fresh deployment or a wiped data directory. The refusal is right for a
participant who brings no document (below); it cannot be the only answer, or the
feature does not work for the case it exists for.

### C. The server fetches the file from the cabinet and seeds the room itself

Rejected: the cabinet contract has no download for the server (`production.md`),
the desktop has no cabinet, and the file is not what the client is looking at —
it lacks the unsaved text the finding is about. It also makes the server's first
join depend on another service being up.

### D. The first writer seeds the room with the document it has open (chosen)

The client that opens the session announces a seed in its auth frame and streams
the document it has open in `seed` frames. The server uses those bytes only when
it has never snapshotted the document:

- the room is built from them (`Room::new` at version 0), so the room's document
  is the client's document;
- the seeding client is answered without a snapshot — it already holds that
  state — and `welcome.resync` stays `false`, so it keeps its document, its
  selection and its undo history;
- every other client joins the room normally and receives the seeded snapshot.

A join to a document with no stored state and no seed is refused `invalid`
(not retryable), and so is a seed from a role that may not write. The room is
never created from `Document::new()`.

## Decision

**D.**

The seed is the model JSON the server already snapshots (`serde_json` of
`wordcraft_doc::Document`), so a seeded room and a restored room are the same
kind of state and a seeded room persists like any other. The handshake is:

```text
C→S  {"type":"auth","token":"…","client_id":"alice","last_version":null,"seed":true}
C→S  {"type":"seed","bytes":[…],"last":false}
C→S  {"type":"seed","bytes":[…],"last":true}
S→C  {"type":"welcome","version":0,…,"resync":false}
S→C  {"type":"nodes","nodes":[…]}
```

- `auth.seed` announces the stream; it is `false` by default, so a client built
  before this step (and the auth frames in older tests) parse unchanged. Such a
  client cannot open a room this server has no state for; it is refused instead
  of being handed an empty document.
- The seed is read only between an auth that announced it and the welcome, and
  only after the token is verified: an unauthenticated peer cannot make the
  server hold a document. A `seed` frame at any other time is refused `invalid`.
- The stream is bounded by `MAX_MODEL` (64 MiB, the store's bound) and by
  `AUTH_TIMEOUT` per frame: a seed that would not fit a snapshot is not a room
  state. A frame that is not a seed, or a stream that stops before `last`,
  refuses the handshake.
- Only a role that `can_write` seeds. A read-only session arriving first is
  refused and waits for the writer that opens the document; the room is created
  once, by the first participant who may change it.
- The client offers the seed while it has accepted nothing (`version == 0`).
  That is exactly the window in which the server may need it, and it keeps the
  offer out of every ordinary reconnect.

The seed is the document as the session started, and the operations after it are
the unconfirmed queue: a client that typed before the socket opened sends the
document it opened plus those operations, so the room ends where the client is —
the replay is not applied twice. An already-open room ignores the seed
completely, so a stale copy cannot overwrite the room it joins.

### What the seed does not carry

The source `.docx` is not in the seed: it is the host's file, and the collab
session does not hold it (the desktop app has a path, the web app gets bytes
from the host page and parses them). A room seeded from a model therefore has no
`source.docx` until its first commit, and a commit before that writes the
modeled form of unchanged paragraphs and says so in `warnings` (ADR-0011 treats
a missing source as a warning, never as text loss). The cabinet's copy still has
the original; the follow-up is a second seed stream for the source, which needs
the host bridge to hand the file bytes to the session.

The seed also does not carry the cabinet's `version`/`etag`. The first commit of
a room sends no `expectedVersion` — the existing limitation in `STATUS.md` §4 —
and seeding does not change it: the clients that can seed do not know the etag
either (the host does). Until a seed can carry it, a room's first commit is
unconditional, exactly as before.

## Consequences

1. **A first join no longer destroys the open document.** The room's document is
   the document the first writer had open; the empty page is gone from every
   path. The text, the formatting the model carries and the client's undo
   history survive the join.
2. **A participant who brings nothing is told so.** A viewer (or a client older
   than this step) that arrives first gets `invalid`, not a blank page: the app
   shows `Failed` with "this document has no state on this server yet; it must be
   opened by a client that may edit it and has the document open". The counter
   `unseeded_joins_total` and one log line name the document; neither carries
   document content.
3. **The protocol has a seeding step.** Rooms are created from state — a stored
   snapshot or a client's copy — and never from a default. The rule that keeps
   finding 2 closed and this one closed is the same rule applied to the other
   branch: a room is only ever the document.
4. **The first writer wins.** If two clients open the same document against a
   server that has no state for it, the first one to authenticate seeds the room
   and the second adopts it, exactly as a client adopts a room that already
   existed. The commit conflict check (M87) is what protects the cabinet's copy
   from being overwritten by a room that started from a stale copy.
5. **A lost snapshot is recoverable by reopening.** If the server loses a room's
   snapshot (a crash before the first snapshot) while clients are connected, the
   reconnecting clients have `version > 0` and therefore do not seed; their join
   is refused as unseeded. Reopening the document opens a new session with
   `version 0`, which seeds. The refusal is loud instead of the silent blank
   room; it is the same trade as §5.30 and the same follow-up (a client that
   does not reconnect in a loop on a non-retryable refusal).

## Tests

- `apps/collab-server/src/tests.rs`:
  - `a_client_with_an_open_document_seeds_the_room_it_joins` — a client joins
    with a document holding text, gets `welcome{version: 0, resync: false}` with
    no snapshot, and a second client's snapshot contains that text; an edit at
    the seeded paragraph's offset lands at version 1.
  - `a_join_without_a_seed_is_refused_when_nothing_is_stored` — `invalid`,
    `retryable: false`, no room, `unseeded_joins_total` counts the refusal; a
    viewer with a copy is refused the same way.
  - `a_new_document_still_opens_a_room` — the blank-document path a new file
    takes: a room at version 0 and ordinary editing after it.
  - `a_seed_does_not_replace_a_stored_snapshot` — a restored room sends its own
    snapshot and the offered copy never reaches it.
  - `a_seed_that_is_not_a_document_is_refused` — the handshake ends without a
    room.
  - `a_viewer_cannot_write` now opens the room with an editor first, because a
    viewer cannot create it.
- `crates/collab/src/wire.rs`: the auth with and without `seed` (an older client
  still parses), and a `seed` frame round trip.
- `crates/collab-client/src/tests.rs`: the frames split at `SEED_CHUNK`, exactly
  one carries `last`, and the parts rebuild the model.
- `crates/collab-ws/tests/reconnect.rs`:
  `the_first_connection_offers_the_open_document` — the transport sends
  `auth{seed: true}` and the document, and the fake server rebuilds it byte for
  byte.
