# WordCraft collaboration: production notes

This page is the operator's view of the collaboration feature: how a room is
deployed, what it promises about the stored file, and what it deliberately does
not do yet. The protocol is in [`rfc.md`](rfc.md); the implementation state, test
evidence and the next session are in [`STATUS.md`](STATUS.md); the shared
decisions are in `.craft/protocol/ARCHITECTURE.md` and ADR 0001–0011.

## Topology

```text
cabinet (mail.lzmkm.ru)
  │  ticket (JWT), session control
  ▼
ingress  ── TLS termination, WebSocket upgrade, body limit
  │
  ▼
wordcraft-collab-server (apps/collab-server)
  │  one room per fileId; authoritative Document + journal
  │
  └─ POST /api/v1/internal/office/sessions/{sessionId}/commit ──▶ cabinet
       Authorization: Bearer <service token>
       {fileId, bytes(base64), sha256, version, expectedVersion?}
       ← {version, etag} | 409 | 401 | 413
```

- Desktop: `wordcraft` opens a WebSocket to the server named by
  `collab.enable`; the JWT comes from `WORDCRAFT_COLLAB_TOKEN` or the host
  bridge.
- Web: the same app behind `craft-web-host`; `wordcraft:session` carries
  `sessionId`, `wsUrl`, the JWT and the role (`host-bridge.md`).
- One room per **fileId**, which is the JWT `doc` claim. The cabinet's callback
  ids are `sid` (path) and `doc` (body), so the ticket the cabinet mints must
  carry the cabinet file id in `doc` — a real deployment uses the numeric file
  id (`"42"`), not the development room key (`"doc-1"`). The cabinet refuses a
  non-numeric `fileId` with `400`.
- The server speaks plain HTTP/WS on a private network; the ingress terminates
  TLS. The commit callback is therefore `http://` only (see "Callback
  contract").

## Server

Binary: `wordcraft-collab-server`.

| Variable | Meaning | Default |
|---|---|---|
| `COLLAB_BIND` | listen address | `127.0.0.1:8087` |
| `COLLAB_JWT_SECRET` | HS256 secret, local runs only | — |
| `COLLAB_JWT_PEM` | cabinet public key (ES256), production | — |
| `COLLAB_DATA_DIR` | snapshots: model, source `.docx`, `opId` window, cabinet etag | `collab-data` |
| `COLLAB_LEASE_TTL_SECS` | silence before another cabinet session may take the lease | `120` |
| `COLLAB_CABINET_URL` | commit callback, `{sessionId}` substituted; unset keeps the commit server-side | — |
| `COLLAB_CABINET_TOKEN` | service token the callback authenticates with (secret) | — |
| `COLLAB_CABINET_TIMEOUT_MS` | deadline for one callback request | `10000` |
| `COLLAB_CABINET_ATTEMPTS` | attempts for a transient failure | `3` |

Exactly one of `COLLAB_JWT_SECRET` / `COLLAB_JWT_PEM` is required.
`COLLAB_CABINET_TOKEN` is required with `COLLAB_CABINET_URL`; the server refuses
to start otherwise, and it refuses a URL that is not `http://` or that has no
`{sessionId}`. The token is read once, kept in a value whose `Debug` prints
`<redacted>`, and never appears in a log line, a metric, a URL or a snapshot.

Endpoints: `GET /healthz`, `GET /metrics` (Prometheus text), `GET /ws`. The
first WebSocket frame must be `auth`.

### Metrics

| Series | Meaning |
|---|---|
| `wordcraft_collab_rooms` | open rooms |
| `wordcraft_collab_connections_total` | accepted WebSocket connections |
| `wordcraft_collab_ops_applied_total` | accepted operations |
| `wordcraft_collab_rejects_total` | refused operations |
| `wordcraft_collab_commits_total` | commits the room built |
| `wordcraft_collab_cabinet_published_total` | commits the cabinet stored |
| `wordcraft_collab_cabinet_conflicts_total` | cabinet `409` answers |
| `wordcraft_collab_cabinet_errors_total` | commits not stored (token, size, exhausted retry) |
| `wordcraft_collab_save_copies_total` | refused copies the owner took to save |
| `wordcraft_collab_presence_total` | presence frames |
| `wordcraft_collab_stale_peers_total` | peers whose outbox overflowed and whose connection was closed (ADR WC-0002) |
| `wordcraft_collab_corrupt_snapshots_total` | joins refused because the stored snapshot cannot be opened (ADR WC-0003) |
| `wordcraft_collab_unseeded_joins_total` | joins refused because the document has no state here and the client brought no copy (ADR WC-0004) |

A publication that is not counted in `cabinet_published_total` did not become a
file version. `cabinet_errors_total` growing while `commits_total` grows is a
callback misconfiguration (often the token or the `fileId` in `doc`).

`corrupt_snapshots_total` growing means a document's stored snapshot cannot be
opened and its joins are refused (ADR WC-0003). The log line names the document
and the file; the document is not lost — restore `model.json`/`meta` (or `prev/`)
from a backup, or remove `<dataDir>/<doc>/` to start it blank on purpose. The
counter is per refused join, so a client that keeps reconnecting keeps counting.

`unseeded_joins_total` growing means a client asked for a document this server
has never snapshotted without offering the document it has open (ADR WC-0004).
That is the case a viewer, or an app older than the seeding step, meets: the
room is not created and no empty page replaces anyone's document. The log line
names the document. The fix is a client that opens the session with the document
open — a writer seeds the room once, and every later join finds it. A data
directory that was wiped or a room whose snapshot never reached the disk shows
the same counter; reopening the document in the app seeds it again.

## Callback contract (mail-frontend M87)

```text
POST {COLLAB_CABINET_URL with {sessionId} and {fileId} substituted}
Authorization: Bearer <COLLAB_CABINET_TOKEN>
Content-Type: application/json

{ "fileId": "42", "name"?: "приказ.docx", "bytes": "<base64 .docx>",
  "sha256": "<hex>", "version": 17, "expectedVersion"?: "1794186677" }

→ 200 { "version": 1794186678, "etag": "1794186678" }
→ 409 the file changed outside the session
→ 401 the callback token is wrong
→ 413 the body is too large
```

- `version` is the room's document version. The cabinet records it in error
  text only; it is never the cabinet's file version.
- `expectedVersion` is the `etag` of the previous accepted commit. The room
  stores it with the snapshot (`meta`: `cabinet_version`, `cabinet_etag`), so a
  server restart does not lose the check. The first commit of a room sends no
  `expectedVersion` and therefore cannot detect an external change.
- `name` is omitted: a disk file keeps its name in the cabinet.
- The answer is delivered to every peer on the socket as
  `committed {version, sha256, warnings, cabinet: {version, etag}}`; the
  participant panel shows the cabinet's version and etag, not the digest.

### What each answer does

| Cabinet answer | Room | Clients |
|---|---|---|
| `200 {version, etag}` | stores the etag with the snapshot, counts `cabinet_published_total` | `committed` with `cabinet` |
| `409` | leaves the etag alone, counts `cabinet_conflicts_total`, logs the refusal, keeps the built file for the owner | every peer gets a retryable `error {code:"cabinetConflict"}` and resyncs; the owner may `saveCopy` and save the room's `.docx` as a copy; the cabinet is not overwritten |
| `401`/`403` | counts `cabinet_errors_total`, logs without the token | the author gets `rejected {code:"forbidden"}`; no retry — fix the token |
| `413` | counts `cabinet_errors_total` | the author gets `rejected {code:"tooLarge"}`; no retry |
| `408`, `425`, `429`, `5xx`, network | retried with the **same body** (`COLLAB_CABINET_ATTEMPTS`, 200 ms then 800 ms) | on success as above; when every attempt fails the author gets `rejected {code:"invalid"}` with the reason and the next Commit retries |
| any other `4xx` | counts `cabinet_errors_total` | the author gets `rejected {code:"invalid"}` |

A `409` is treated as an external edit, as the contract says. Idempotent replay
past a lost `200` is not possible with this contract: a retry carries the old
`expectedVersion`, so a cabinet that did store the first attempt answers `409`
and the room reports the conflict instead of a second version. The cabinet's own
notes list callback idempotency as its next step; until then, a lost answer
after a successful store surfaces as a conflict, and the recovery is saving a
copy of the room's file (the panel's "Save a copy…") or reopening it (the stored
bytes are already the room's).

## Rollout to `CRAFT_COLLAB_ENABLED=true`

1. Build the pinned revision (fork pin from ADR-0016 §3) and the server image;
   push the image to the cabinet's registry.
2. Put the same 64-hex value in the cabinet's `CRAFT_CALLBACK_TOKEN` and the
   room's `COLLAB_CABINET_TOKEN` (`deploy/.env`, mode 0600). Nothing prints it.
3. Point the room at the backend:
   `COLLAB_CABINET_URL=http://backend:8080/api/v1/internal/office/sessions/{sessionId}/commit`.
4. Mint tickets that carry the numeric `fileId` in `doc` and the cabinet session
   in `sid`.
5. Two clients open one file, edit, press Commit; `committed` shows the cabinet
   version, `/files/{id}/versions` shows the previous version, and
   `wordcraft_collab_cabinet_published_total` moves.
6. Only then flip `CRAFT_COLLAB_ENABLED=true`; the feature flag stays off until
   the pinned build and the callback are both in place.

## Rollback

- Flip `CRAFT_COLLAB_ENABLED=false` (single-user editing is untouched by this
  slice).
- Unset `COLLAB_CABINET_URL` on the room: a commit builds the file and returns
  its digest, and the host owns the storage again; existing snapshots keep their
  `cabinet_version`/`cabinet_etag`, so re-enabling continues from the same
  expectation.
- The cabinet's rollback is its own (`docs/CUTOVER.md` §M87): remove
  `CRAFT_CALLBACK_TOKEN`, restore `deploy/.env.bak-m87-*`, recreate the backend.
- No migration is involved on either side.

## Known limits (operator view)

- The callback is `http://`. An `https://` value is refused at startup; put the
  room behind the ingress on the same private network instead.
- A lost `200` is reported as `409` (see "What each answer does"); the room does
  not attempt a second version.
- The first commit after a room is created (or after its snapshot was removed)
  sends no `expectedVersion`, so a file edited outside the session in that
  window is overwritten. Snapshots are kept per document, so this is the
  "session opened" case, not every start.
- One room per fileId; a second cabinet session may join as `owner` or after the
  lease goes silent (ADR-0008).
- `/metrics` is plain text on the private network; no token, document content or
  filename appears in it.
- Logs name the document id and the HTTP status of a refusal; never the token,
  the body or the file content.
