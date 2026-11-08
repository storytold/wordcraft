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

At least one of the two token keys is required; the server refuses to start
without one. The token is never logged.

## Endpoints

- `GET /healthz` — `ok` when the process is serving.
- `GET /metrics` — Prometheus text: rooms, connections, applied operations,
  rejects, commits, presence frames.
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
C→S  {"type":"auth","token":"…","clientId":"alice","lastVersion":41}
S→C  {"type":"welcome","sessionId":"…","role":"editor","version":42,"epoch":3,…}
S→C  {"type":"tail","entries":[…]}      // or snapshot frames, then:
S→C  {"type":"nodes","nodes":[{"id":1,"story":"body","path":[0]}]}
C→S  {"type":"op","op":{"opId":…,"baseVersion":42,"kind":"primitive",
      "target":{"story":"body","nodeId":1,"off":0},"payload":{"op":"insert_text","text":"Hi"}}}
S→C  {"type":"applied","applied":{"opId":…,"version":43,"epoch":3,"delta":{…}}}
C↔S  {"type":"presence","state":{…}}    // ephemeral, never journaled
C→S  {"type":"commit","opId":…,"expectedVersion":43}
S→C  {"type":"committed","version":43,"sha256":"…","warnings":[…]}
```

`nodes` tells the client which session ids it may address; the server sends it
again after a structural edit moves paths (ADR-0002). Operations are addressed
by `(nodeId, off)`; index paths never travel as identity.

## Limits (v1)

- WebSocket frame ≤ 1 MiB.
- Snapshot frames ≤ 512 KiB each.
- A client must authenticate within 10 s.
- Presence is expected at 5–10 Hz; the server does not journal it.
- One room per document; a room is dropped when its last peer leaves.

## Commit

Only `owner` may commit. The server writes `.docx` from the authoritative model
(`wordcraft_docx::write`) and answers with the SHA-256 and round-trip warnings.
`expectedVersion` must match the head, otherwise the client gets
`rejected{code:"conflict"}` (the cabinet stores the new version and keeps the
previous one, ADR-0009).

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
step of several operations is `unsupported` until a transaction exists. A
command has no inverse and acts as a barrier, so undo never skips over it.

## What is not here yet

- Client connection and UI (feature flag `collab`), PR-4.
- Snapshot persistence on disk and restart recovery.
- Lease enforcement and token refresh.
