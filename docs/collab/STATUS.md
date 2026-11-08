# Real-time collaboration: implementation status

- Date: 2026-11-08
- Branch: `feature/realtime-collaboration` (fork `copoka/wordcraft`)
- PR: <https://github.com/storytold/wordcraft/pull/5834> (draft, 24 commits)
- HEAD at this writing: `2b4aab2d`
- Protocol: `rfc.md`; server: `server.md`; run book: `README.md`; host blocker:
  `host-bridge.md`.

This page records what is implemented, how it was verified, what is known to be
missing, and where the next session should start.

## 1. What is implemented

### Protocol (`crates/collab`)

- One envelope per operation: `{opId, baseVersion, epoch, kind, target, payload,
  history}`; the server is the single authority (ADR-0001/0003).
- Journal with `opId` dedup, monotonic `version`, `epoch` for structural work,
  tail replay and compaction (ADR-0003/0005).
- Session node ids with tombstones and path shifts; a fresh node map is sent when
  an operation moves paths or adds ids (ADR-0002).
- Per-user undo/redo as an operation with a `history` field: one version per undo,
  `undoConflict` when the step's target changed, `unsupported` for a step of
  several operations, and commands act as an undo barrier (ADR-0010).
- Role gates: viewer cannot write; commenter may only add/resolve comments
  (including the `review.*Comment` commands); owner alone commits (ADR-0008).
- Snapshot = model JSON + source `.docx`; `Document::kept` is taken back from the
  source on restore (ADR-0011). Server snapshots every 500 operations, on commit
  and when the last peer leaves; a final snapshot is written on SIGTERM.

### Document adapter (`crates/collab-doc`)

- Primitives: `insert_text`, `delete_range`, `replace_range`, `split_para`,
  `merge_para`, `insert_block` (with the block body), `remove_block`.
- Every accepted operation returns an authoritative delta; a replica applies it
  with `apply_delta` and converges to the same document JSON (tested after each
  operation).
- `diff(before, after, ids)` derives a local command's primitives from the
  document difference. Cloning a document shares its `Arc` blocks, so untouched
  blocks are one pointer compare. Text becomes `replace_range`, Enter/Backspace
  become `split_para`/`merge_para`, block insert/remove become structural.
  Formatting-only changes, tables and several moves are reported as unsupported.

### Engine adapter (`crates/collab-engine`)

- `SessionModel` keeps a real `Session`: `kind:"command"` runs through the same
  `Session::run` as the UI, CLI and MCP, with the actor from the JWT as the
  tracked-changes/comment author and an explicit caret or selection from the
  payload.
- `apply_delta` replays primitives into the `Document` and commands through
  `Session::run_remote`; remote edits do not become a local undo step.
- `run_remote` starts a fresh undo group and truncates the step it would add, so
  another user's typing never merges into or pollutes this user's history.
- Command deny-list (`file.*`, `tools.*`, `ui.*`, `view.*`, `caret.*`, `select.*`,
  `collab.*`, undo/redo/repeat, read-aloud) and only mutating commands.
- A command that grows the document/parts (tables, comments, shapes, references)
  reports `nodes changed`, so replicas refresh the node map.

### Client and transport (`crates/collab-client`, `crates/collab-ws`)

- Operation ids, unconfirmed queue, per-client undo/redo mirrors, duplicate
  detection after reconnect, resync on a stale/retryable refusal and on a version
  gap.
- WebSocket task: auth with `lastVersion`, welcome, tail/snapshot, presence
  (throttled by the app), commit, resync and token renewal. A bounded server
  queue can drop messages; the client notices the version gap and requests a
  snapshot instead of applying a partial history.
- Socket-level reconnect test: the server drops the socket after reading an
  operation; the client reconnects and resends the same `opId`, and the applied
  answer reaches the app.

### App and UI (`crates/collab-app`, `crates/ui-egui`)

- `Collab` owns the transport, node map, participants and presence; connects to
  the address stored by `collab.enable`, with the JWT from
  `WORDCRAFT_COLLAB_TOKEN` (never in prefs, a URL or the log).
- `WordApp::execute` is wrapped: read-only roles and a resynchronizing session
  refuse document commands before the engine runs; a local command is diffed and
  sent; `edit.undo`/`edit.redo` go to the room instead of the local snapshot
  history.
- Participants window: status, role, participants, commit button (owner) and the
  commit result/warnings. Remote carets **and selection shading** are drawn on the
  canvas in the participant's colour.
- `collab.disable` drops the session; with the flag off nothing opens a socket.
- Saved preferences plus a token in the environment resume the session on start.

### Server (`apps/collab-server`)

- `tokio`/`axum` WebSocket, JWT first message (HS256 dev / ES256 public key),
  `aud`/`app`/`role` checks, `exp` re-checked on **every** frame.
- One room per document with a lease; a different cabinet session may join as
  owner or after the old session has been silent for `COLLAB_LEASE_TTL_SECS`
  (default 120) and has no connected peer.
- Presence is fanned out to the other participants and never journaled; a viewer
  may send it. Presence is not echoed to its author.
- Commit is owner-only, checks `expectedVersion`, writes the `.docx` in memory,
  returns SHA-256 and warnings, and stores the source bytes for the next snapshot.
- Limits: 8 participants, 200 operations/s per room, 1 MiB frame, 64 MiB document;
  bounded per-peer queues (512).
- `GET /healthz`, `GET /metrics`; logs contain no document content, payloads or
  tokens.
- Graceful shutdown (SIGTERM/SIGINT) writes every open room before exit.
- Snapshot restore takes `kept` markup from the source `.docx` (ADR-0011).

## 2. Acceptance criteria (TASK.md)

| Criterion | Status | Evidence |
|---|---|---|
| Two or more clients edit one `.docx` | done | `two_clients_edit_through_the_server`, delta convergence tests |
| Cursors, selections, participants visible | done | participants panel, remote carets + selection shading |
| Changes converge without loss or duplicates | done | `a_replica_applies_the_deltas_and_converges`, socket tests |
| Reconnect and resync | done | `a_reconnecting_client_catches_up_from_the_journal`, `reconnect.rs`, gap→snapshot |
| Per-user undo/redo | done | `undo_and_redo_over_the_socket_change_the_committed_text`, `undo_round_trips_through_the_room` |
| Viewer cannot edit | done | `a_viewer_cannot_write` + client gate |
| Commit creates a new version, original kept | server side done | commit/409/`warnings[]`; cabinet owns versions |
| Single-user mode intact; feature behind a flag | done | flag off by default; 1137 UI tests |
| RFC, protocol, tests, documentation | done | `rfc.md`, `server.md`, `README.md`, this page |
| PR to upstream | open (draft) | PR #5834; GitHub CI waits for maintainer approval |
| Web host bridge | **blocked outside this repo** | see `host-bridge.md` |

## 3. Verification

Run from `upstream/`:

```bash
cargo test -p wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine \
  -p wordcraft-collab-client -p wordcraft-collab-ws -p wordcraft-collab-app \
  -p wordcraft-collab-server          # 107 tests, 1 ignored (LibreOffice gate)
cargo test -p wordcraft-collab-server -- --ignored   # commit → LibreOffice
cargo test -p wordcraft-engine collab::              # 5 flag/remote tests
cargo test -p wordcraft-ui-egui --lib                # 1137 tests
cargo xtask fmt / clippy / test / doctest / assets / layers / wasm
```

Results at this writing:

- 107 protocol/adapter/client/server tests + 1 LibreOffice gate, all green.
- 5 engine collaboration tests, all green.
- 1137 UI tests, green.
- Workspace `cargo clippy --all-targets -- -D warnings`, fmt, doctest, assets
  (48 files), layers (26 crates) and the wasm32 check (21 crates) are green.
- The workspace test step excludes three proofing tests that fail on `main` in
  this environment too (French fallback marks text without a French dictionary);
  with `CRAFT_I18N_DIR` pointing at the pinned `craft-i18n` checkout the Swiss
  language test passes.

## 4. Known limitations

- **Web host bridge** (the only production blocker for the cabinet): `craft-web-host`
  in `storytold/craft-libs` owns a closed message enum without
  `session/session-state/presence/commit`. The exact patch and the app-side plan
  are in `host-bridge.md`. The web build therefore stays single-user; the desktop
  build connects directly.
- A step of several operations cannot be undone yet (`unsupported`); commands have
  no inverse and act as an undo barrier.
- A command that allocates ids (comments, tables, shapes, footnotes…) makes
  replicas request a snapshot after it; there is no id delta on the wire yet.
- Node ids address top-level paragraphs and story parts; a table cell has no id,
  so table-cell edits travel as commands and may resync.
- Offline editing is not supported: while a session reconnects, document commands
  pause instead of diverging.
- The token is read from the environment on desktop; the cabinet must push a fresh
  token through the host bridge when it arrives.
- No two-machine manual GUI test; the UI tests are headless.

## 5. Next session

1. **Unblock the web path** (requires a change in `storytold/craft-libs`):
   add the five messages from `host-bridge.md`, bump/ship the crate, then add the
   wasm transport and the `Message::Session` → `Collab` wiring in `apps/wordcraft-web`.
2. Mark PR #5834 ready once the host-bridge item is resolved and CI has run
   (GitHub reports `action_required` for fork PRs until a maintainer approves).
3. Optional hardening: multi-operation undo transaction (apply all inverses or
   none), command inverses for the common engine commands, table-cell node ids and
   property primitives, offline queue.
4. Manual acceptance: two desktops over the real server, 100+ page document,
   commit and reopen in Word/LibreOffice.

## 6. Where things live

| Concern | File |
|---|---|
| Wire types, journal, room, undo | `crates/collab/src/{wire,journal,room,undo,ids}.rs` |
| Primitives, delta replay, diff | `crates/collab-doc/src/lib.rs` |
| Session commands, remote replay | `crates/collab-engine/src/lib.rs` |
| Client state machine | `crates/collab-client/src/lib.rs` |
| Socket transport | `crates/collab-ws/src/lib.rs` |
| Desktop session | `crates/collab-app/src/lib.rs` |
| UI wiring, panel, carets | `crates/ui-egui/src/collab.rs`, `app/commands.rs` |
| Server | `apps/collab-server/src/{lib,main,store}.rs` |
| Run book | `docs/collab/README.md` |
