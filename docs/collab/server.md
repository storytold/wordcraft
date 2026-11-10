# Running the WordCraft sync server

`apps/collab-server` is the authoritative sync server for real-time editing
(ADR-0001, ADR-0007). It is a separate native process; the desktop and web apps
connect to it over WebSocket. The feature is off by default in the client.

## Build and run

```bash
cargo build -p wordcraft-collab-server
COLLAB_BIND=127.0.0.1:8087 \
COLLAB_JWT_SECRET=dev-secret \
cargo run -p wordcraft-collab-server
```

| Variable | Meaning | Default |
|---|---|---|
| `COLLAB_BIND` | address to listen on | `127.0.0.1:8087` |
| `COLLAB_JWT_SECRET` | HS256 shared secret (local development only) | — |
| `COLLAB_JWT_PEM` | ES256 public key PEM (production, ADR-0008) | — |
| `COLLAB_DATA_DIR` | snapshots (model, source `.docx`, `opId` window, cabinet etag) | `collab-data` |
| `COLLAB_LEASE_TTL_SECS` | silence before another cabinet session may take the lease | `120` |
| `COLLAB_CABINET_URL` | commit callback, `{sessionId}` substituted; unset keeps the commit server-side | — |
| `COLLAB_CABINET_TOKEN` | service token of the callback (secret; never logged) | — |
| `COLLAB_CABINET_TIMEOUT_MS` | deadline for one callback request | `10000` |
| `COLLAB_CABINET_ATTEMPTS` | attempts for a transient callback failure | `3` |

At least one of the two token keys is required; the server refuses to start
without one. The token is never logged.

## Endpoints

- `GET /healthz` — `ok` when the process is serving.
- `GET /metrics` — Prometheus text: rooms, connections, applied operations,
  rejects, commits, presence frames, stale peers.
- `GET /ws` — WebSocket; the first frame must be `auth`.

## Development token

```bash
COLLAB_JWT_SECRET=dev-secret \
cargo run -p wordcraft-collab-server --example dev-token -- editor alice doc-1
```

The claims are `{sub, aud:"collab", doc, app:"wordcraft", role, sid, iat, exp}`
(ADR-0008). Production tokens come from the cabinet and are signed with ES256.

## Session flow

```text
C→S  {"type":"auth","token":"…","client_id":"alice","last_version":41}
S→C  {"type":"welcome","session_id":"…","role":"editor","version":42,"epoch":3,…}
S→C  {"type":"tail","entries":[…]}      // or snapshot frames, then:
S→C  {"type":"nodes","nodes":[{"id":1,"story":"body","path":[0]}]}
C→S  {"type":"op","op":{"opId":…,"baseVersion":42,"kind":"primitive",
      "target":{"story":"body","nodeId":1,"off":0},"payload":{"op":"insert_text","text":"Hi"}}}
S→C  {"type":"applied","applied":{"opId":…,"version":43,"epoch":3,"delta":{…}}}
C↔S  {"type":"presence","client_id":"alice","role":"editor","state":{…}}
C→S  {"type":"commit","op_id":…,"expected_version":43}
S→C  {"type":"committed","version":43,"sha256":"…","warnings":[…]}
```

The envelope structs (`op`, `applied`, `reject`, `target`) keep camelCase
(ADR-0001); the fields of a `clientMsg`/`serverMsg` variant itself are
snake_case, because serde's `rename_all` on an enum renames variants, not their
fields. `presence` is the exception that proves the rule: the server adds the
sender's `role` (from the JWT, never from the client) and the author's frame is
not echoed back.

`nodes` tells the client which session ids it may address; the server sends it
again after a structural edit moves paths (ADR-0002). Operations are addressed
by `(nodeId, off)`; index paths never travel as identity.

**A room is never started blank** (ADR WC-0004). A client that opens a document
this server has never snapshotted brings its copy: it sets `seed` in the auth
frame and streams the document it has open, in the same JSON the server
snapshots:

```text
C→S  {"type":"auth","token":"…","client_id":"alice","last_version":null,"seed":true}
C→S  {"type":"seed","bytes":[…],"last":false}
C→S  {"type":"seed","bytes":[…],"last":true}
S→C  {"type":"welcome","session_id":"…","role":"editor","version":0,"epoch":0,…,"resync":false}
S→C  {"type":"nodes","nodes":[…]}
```

The room is built from those bytes, so its first operation lands on the document
the client is looking at. The seeding client is answered without a snapshot — it
already holds that state — and keeps its document, selection and undo history. A
join to a document with no stored state and no seed is refused `invalid` (not
retryable) and opens no room; so is a seed from a viewer. A room that exists
ignores the seed and answers with its snapshot, so a stale copy can never
replace it.

## Snapshots on disk

A room is snapshotted every 500 operations, after a commit, when its last peer
leaves and on SIGTERM. The directory `<dataDir>/<doc>/` holds:

| File | Content |
|---|---|
| `model.json` | `serde_json` of `wordcraft_doc::Document` |
| `source.docx` | the file the model was read from, for `Document::kept` (ADR-0011) |
| `dedup.json` | the `opId` window: stored `applied` answers at or before the snapshot version |
| `meta` | `version=` and `epoch=` lines, written last as the commit marker |
| `prev/` | the previous set of the four files, for one-step rollback |

The window makes a retry survive a restart: a client that reconnects with the
same `opId` after the server came back is answered with the stored `applied`
frame and the version does not move. It is server state only — the wire is
unchanged. A snapshot written before the window was stored has no `dedup.json`
and still opens; only deduplication across that restart is missing. A retry of
an operation accepted after the last snapshot is not covered, because the restore
cannot replay the operation itself either.

**A snapshot that cannot be opened refuses the document** (ADR WC-0003). On the
first join the store tells three states apart:

| State | When | What the server does |
|---|---|---|
| absent | there is no `model.json` | the room is built from the copy the joining writer offers (ADR WC-0004); a join that offers none is refused `invalid` |
| opens | the model is a `wordcraft_doc::Document` within the model bound and every known `meta` line parses | the room is restored from it |
| damaged | `model.json` or `meta` is unreadable, `meta` has no `version`, a known `meta` value is not a number, a cabinet marker is half-written, the model does not parse or is over the bound | the join is refused `invalid` (not retryable), no room is created, nothing is written over the files |

The absent case no longer means "an empty document": the room is the copy the
first writer brings, and a client that brings none is refused rather than shown
a blank page in place of its document (`unseeded_joins_total`).

`dedup.json` and `source.docx` stay forgiving: a damaged window costs a
duplicated answer across a restart and a missing source costs original markup on
unchanged paragraphs; neither is text. The refusal leaves the files as they are,
including the `prev/` pair, and counts in `corrupt_snapshots_total`. To recover,
restore `model.json`/`meta` (or `prev/`) from a backup; to accept the loss and
start a blank document on purpose, remove `<dataDir>/<doc>/`.

## Limits (v1)

- WebSocket frame ≤ 1 MiB.
- Snapshot frames ≤ 512 KiB each; seed frames ≤ 128 KiB each (a byte array is
  JSON-encoded as numbers, so a chunk stays well inside one frame).
- A client must authenticate within 10 s, and send each seed frame within 10 s
  once it announced a stream.
- A seed is bounded by the 64 MiB model bound; a larger document is refused
  rather than held.
- Presence is expected at 5–10 Hz; the server does not journal it.
- One room per document; a room is dropped when its last peer leaves.
- A peer's outbox holds 512 frames (`Config::peer_queue`). A peer that stops
  reading overflows it: the operation it missed is counted in
  `stale_peers_total`, the peer is marked stale and its connection ends, so the
  client reconnects with `lastVersion` and resyncs (ADR WC-0002). Every write to
  a peer is bounded by the heartbeat period (10 s), so a stalled socket is
  closed even when no further frame arrives.

## Commit

Only `owner` may commit. The server writes `.docx` from the authoritative model
(`wordcraft_docx::write`) and answers with the SHA-256 and round-trip warnings.
`expected_version` must match the head, otherwise the client gets
`rejected{code:"conflict"}` (the cabinet stores the new version and keeps the
previous one, ADR-0009).

With `COLLAB_CABINET_URL` set, the room also becomes an HTTP client of the
cabinet (`cabinet.rs`, mail-frontend M87): it posts
`{fileId, name?, bytes(base64), sha256, version, expectedVersion?}` — `fileId`
and `sessionId` come from the JWT's `doc` and `sid` claims — and the cabinet
answers `{version, etag}`. `committed` then carries that copy
(`cabinet:{version, etag}`) to every peer, and the etag is stored with the
snapshot so the next commit sends it as `expectedVersion` even after a restart.
`409` (the file changed outside the session) becomes a retryable
`error{code:"cabinetConflict"}` to every peer so the replicas resync; `401`/`403`
and `413` are one refusal to the author with no retry; a network failure or a
`5xx`/`408`/`429` is retried with the same body. The callback accepts `http://`
only: TLS is terminated by the ingress (`production.md`).

The room keeps the file it built for a refused commit in memory. The owner may
send `saveCopy{op_id}` to fetch it as `saveCopy{version, sha256, warnings[],
bytes, last}` chunks (512 KiB, like a snapshot) and save a copy without writing
the cabinet (TASK.md §6). Editors are refused (`forbidden`); with no refused
commit the answer is `rejected{code:"conflict"}`. The lookalike `saveCopy` frame
in both directions is deliberate: the direction says who sends it. A restart
drops the copy (it is not part of the document or the snapshot); the owner
commits again to see the new conflict.

Known limitation, ADR-0005: `Document::kept` is not serialized, so unmodelled
paragraph markup is preserved only for paragraphs unchanged since open. The
warning says so; the fix is to keep the source `.docx` next to the snapshot.

## Command operations

Besides the positional primitives, the server accepts `kind:"command"` operations
for engine commands a client cannot express as primitives. The room model wraps a
real `Session` (`crates/collab-engine`), so a command runs through the same
`Session::run` the UI uses:

```json
{"kind":"command",
 "target":{"story":"body","nodeId":18,"off":42},
 "payload":{"command":"text.insert","params":{"text":"Hi"}}}
```

The caret comes from `target`, `author` comes from the JWT `sub`, and the
response delta repeats the command so other replicas can run it at the same
place. Only mutating commands outside areas that reach beyond the document are
accepted: `file.`, `tools.`, `ui.`, `view.`, `caret.`, `select.`, `collab.`,
`edit.undo`, `edit.redo`, `edit.repeat` and `review.readAloud` are refused with
`forbidden`. The server session keeps no undo history: per-user undo is the
protocol's (ADR-0010), and a command has no inverse there.

## Presence

`presence` is ephemeral: it is not journaled, does not move `version`, and is
sent to the **other** participants — the author's own caret is already in its
window. Cursors and selections are addresses (`story`, `nodeId`, `off`), never
text. A viewer sends presence but no operations.

A successful join is announced with the same frame: right after `welcome`, the
server sends the peers already in the room a `presence` with an empty `state`,
the newcomer's `client_id` and its role from the JWT. Their participant list
shows the role before the newcomer's first presence frame; the newcomer is not
echoed its own join and learns the existing peers from `welcome`. A client
cannot name a role, so nothing on the wire is trusted for it.

## Roles

The server checks the role on every operation. `viewer` is refused before the
model; `commenter` may only send `add_comment`/`resolve_comment` primitives (and
merge-class annotations), never document text; `owner` is the only role that
commits. The client refuses document commands for a viewer too, but that is UX:
the server is the authority.

## Undo and redo

The client moves its own history by sending an ordinary operation with a
`history` field (ADR-0010); there is no separate message:

```json
{"type":"op","op":{"opId":{"client":"alice","seq":7},"baseVersion":3,"kind":"primitive",
 "target":{"story":"body","nodeId":18,"off":0},"payload":null,
 "history":{"action":"undo","opId":{"client":"alice","seq":6}}}}
```

The server takes only the actor's newest step, verifies the named step is that
one, and replays its inverse operations under the request. The request gets one
version; the replayed operations are not journaled separately. A target touched
after the step yields `rejected{code:"undoConflict"}` and changes nothing. A
step of several operations replays as one transaction: either every inverse
lands or the ones already applied are taken back, and the version does not move.
Replicas get the step as one `{"op":"transaction","ops":[…]}` delta and apply it
on a scratch copy. A command has no inverse and acts as a barrier, so undo never
skips over it.

## What is not here yet

- Client connection and UI (feature flag `collab`), PR-4.
- Lease enforcement and token refresh.
