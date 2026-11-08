# Running a collaborative WordCraft session

> Implementation status, verification results and the remaining web blocker:
> [`STATUS.md`](STATUS.md).

The sync server is `wordcraft-collab-server` (this workspace); the desktop app
connects to it when the `collab` preference is on. The web app needs the cabinet's
host bridge first (`host-bridge.md`). Nothing opens a socket while the flag is off.

## 1. Start the server

```bash
COLLAB_BIND=127.0.0.1:8087 \
COLLAB_JWT_SECRET=dev-secret \
COLLAB_DATA_DIR=/var/lib/wordcraft-collab \
cargo run -p wordcraft-collab-server
```

- `COLLAB_JWT_SECRET` — HS256 secret for local development.
- `COLLAB_JWT_PEM` — ES256 **public** key for production (ADR-0008); set one of the two.
- `COLLAB_DATA_DIR` — snapshots (`model.json`, `source.docx`, `meta`, one previous pair).
- `COLLAB_LEASE_TTL_SECS` — how long a cabinet session may be silent before another
  session may take the lease (default 120).
- `GET /healthz`, `GET /metrics`, WebSocket at `/ws`. No token is logged. `exp` is
  checked on every frame, not only at the handshake; a running session therefore
  gets `tokenExpired` and must reconnect with a fresh token. SIGTERM/SIGINT writes
  every open room before exit.

## 2. Mint a session token

Claims (base64url, `HS256` with the dev secret or `ES256` with the cabinet key):

```json
{"sub":"alice","aud":"collab","doc":"doc-1","app":"wordcraft",
 "role":"editor","sid":"session-1","iat":1762000000,"exp":1762003600}
```

`role` ∈ `owner | editor | commenter | viewer`. `doc` and `app` are checked
against the request; `sid` is the lease. A token for another app or document is
refused, and an expired token makes the client resync.

## 3. Enable collaboration in the app

```bash
WORDCRAFT_COLLAB_TOKEN=<jwt> cargo run -p wordcraft
```

then Review › Collaborate › `collab.enable {"server":"ws://127.0.0.1:8087/ws"}`.
The token comes from the environment (or, in the cabinet, from the host bridge);
it is not stored in preferences, a URL or the document. `collab.disable` closes
the session; single-user mode is unchanged.

The same session can be driven headlessly through the control channel or MCP:

```json
{"id":1,"method":"engine.execute","params":{"command":"collab.enable","params":{"server":"ws://127.0.0.1:8087/ws"}}}
```

## 4. What is replicated

- Typing, deletion, paragraph split/join and block insert/remove: derived from
  the document difference and sent as primitives (one operation per user action).
- Formatting, tables and complex commands: sent as `kind:"command"` with the
  explicit selection; the server runs the same engine command and every replica
  replays its delta.
- Per-user undo/redo (one version per undo, conflicts refused), presence and
  participants, owner-only commit, snapshots every 500 operations (model JSON
  plus the source `.docx`, ADR-0011).
- A viewer's document commands are refused by the client before the engine and
  again by the server.

## 5. Limits (v1)

- Commands that read local state (clipboard, mouse drag, file dialogs) run in the
  single-user path only; the app does not send them.
- A table edit is sent as a command; if the server refuses it the session asks
  for a snapshot instead of guessing.
- A step of several operations replays as one transaction (all inverses or
  none); commands have no inverse and act as an undo barrier.
- `kept`/`passthrough` are not in the protocol: the authoritative server keeps
  the source `.docx` next to the model, and commit says what it could not keep.
- Web: blocked on `craft-web-host` messages (`host-bridge.md`).

## 6. Tests

```bash
cargo test -p wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine \
  -p wordcraft-collab-client -p wordcraft-collab-ws -p wordcraft-collab-app \
  -p wordcraft-collab-server
cargo test -p wordcraft-collab-server -- --ignored   # LibreOffice commit gate
```

The server tests drive two clients over a real WebSocket, including reconnect,
presence, viewer refusal, undo/redo and a commit that LibreOffice opens.
