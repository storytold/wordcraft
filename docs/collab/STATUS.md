# Real-time collaboration: implementation status

- Date: 2026-11-09
- Branch: `feature/realtime-collaboration` (fork `copoka/wordcraft`)
- PR: <https://github.com/storytold/wordcraft/pull/5834> (draft)
- HEAD at this writing: the two-client host-scenario fix on top of the
  craft-libs pin (`376231e1`)
- Protocol: `rfc.md`; server: `server.md`; run book: `README.md`; host bridge:
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
  `undoConflict` when the step's target changed, a step of several operations
  replayed as one all-or-nothing transaction (a refused inverse takes back the
  ones already applied), and commands act as an undo barrier (ADR-0010).
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
- A client id belongs to a session, not to a document:
  `wordcraft_collab_client::session_client_id` mixes a native suffix (process id,
  clock, counter) or a browser random one into `ui-<doc>`/`web-<doc>`. Before, two
  windows on `Document1` both called themselves `web-1`/`ui-1`, one replaced the
  other in the room and their `(clientId, seq)` operation ids collided (§5.1).
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

### Web session (`crates/collab-web`, `apps/wordcraft-web`)

- The transport-free half of `wordcraft-collab-app` (`SyncState`, `Status`,
  cursors, the participant view) now builds for wasm too; only the native
  `Collab` is desktop-only.
- `wordcraft-collab-web::Collab` drives the same `wordcraft-collab-client`
  state machine over the browser's `WebSocket` (no threads; callbacks fill a
  queue that `poll` drains each frame). Reconnect backs off 250 ms → 10 s and
  resends unconfirmed operations with their ids, exactly as the desktop
  transport.
- `apps/wordcraft-web/src/host.rs` handles the extended contract
  (`host-bridge.md`): on `wordcraft:session` it stores `session_id`, `ws_url`,
  the JWT and the role in `WordApp` **first**, then connects; the token is never
  put in the URL, a log, `localStorage` or preferences. On
  `wordcraft:committed` it records the cabinet's version and etag and shows the
  result.
- `session-state` is posted when the sync status or the participant list
  changes (join, leave, reconnect); `presence` is coalesced to 10 Hz (ADR-0006).
  Without `wordcraft:session` the app makes no network call and the `collab`
  flag stays off: the web build is single-user until the cabinet opens a session.
- When the host owns the file (a session came through the bridge), the Commit
  button sends `wordcraft:commit` (no `bytes`: the cabinet fetches the assembled
  file itself, as the contract's `bytes?` allows) and the panel shows the
  `wordcraft:committed` answer; otherwise the sync server commits and its
  sha256/warnings show in the panel.

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
| Web host bridge | done; `craft-web-host` pinned to the fork revision `7666e2b1` | `host.rs`, `collab-web`, `host-bridge.md` |
| Two clients through a cabinet-like host page | done headlessly: one browser, two iframes | §5.1: session → edit → presence → commit → `committed`; 30 ops, 0 rejects |
| Two real machines and the real cabinet storage | not run | §5.1 "What remains for a person" |

## 3. Verification

Run from `upstream/`:

```bash
cargo test -p wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine \
  -p wordcraft-collab-client -p wordcraft-collab-ws -p wordcraft-collab-app \
  -p wordcraft-collab-web -p wordcraft-web \
  -p wordcraft-collab-server          # 113 unit + 1 socket test, 1 ignored (LibreOffice gate)
cargo test -p wordcraft-collab-server -- --ignored   # commit → LibreOffice
cargo test -p wordcraft-engine collab::              # 5 flag/remote tests
cargo test -p wordcraft-ui-egui --lib                # 1137 tests
cargo xtask fmt / clippy / test / doctest / assets / layers / wasm
```

Results at this writing:

- 120 protocol/adapter/client/server tests (119 unit + 1 socket reconnect), all
  green; the LibreOffice commit gate stays the one ignored test. One of the unit
  tests is new: `two_sessions_on_one_document_get_different_client_ids`.
- 5 engine collaboration tests, all green.
- 1137 UI tests, green (3 ignored).
- 5 web-session tests (`wordcraft-collab-web`: session/token redaction, commit
  result, single-user default, status words, participant mapping) and 9
  `wordcraft-web` tests (3 new host-routing tests plus the existing prefs/save
  ones), all green.
- `cargo clippy --all-targets -- -D warnings` on `collab-client`, `collab-app`,
  `collab-web`, `ui-egui` and `wordcraft-web`, `cargo fmt --all --check`,
  `cargo xtask layers` (27 crates) and `cargo check -p wordcraft-web --target
  wasm32-unknown-unknown` were re-run for this slice on the `agents` build host
  and are green.
- These runs used `craft-web-host` from a path to the `craft-libs`
  `feat/collab-session-messages` checkout; the manifest now pins that branch as
  a git dependency of the `copoka` fork (see §4).
- With the git pin in place, `cargo check -p wordcraft-web --target
  wasm32-unknown-unknown`, `cargo xtask wasm` (22 crates), `cargo fmt --all
  --check`, `cargo clippy -p wordcraft-collab-web -p wordcraft-web --all-targets
  -- -D warnings` and `cargo test -p wordcraft-collab-web -p wordcraft-web`
  (14 tests) run from the committed manifest, green.
- `trunk build` in `apps/wordcraft-web` (trunk 0.22.1) finished on the pin and
  wrote `dist/web` (`index.html`, the JS glue, a 76.9 MB dev `_bg.wasm`).
- Two-client host run (2026-11-09): Chrome 154 headless (SwiftShader WebGL), the
  same `dist/web` served from one origin, two iframes hosted by a page that
  speaks the `craft-web-host` contract, and the sync server on HS256 dev tokens
  with one `sid`. Both clients synchronized, 30 operations accepted with 0
  rejects, the same text on both screens and in the server snapshot (version
  30), presence from both, the owner's commit posted and the cabinet's
  `committed` answered. Details and limits: §5.1.
- The workspace test step excludes three proofing tests that fail on `main` in
  this environment too (French fallback marks text without a French dictionary);
  with `CRAFT_I18N_DIR` pointing at the pinned `craft-i18n` checkout the Swiss
  language test passes.

## 4. Known limitations

- **The craft-libs pin is a fork revision.** `crates/web-host` in
  `storytold/craft-libs` has the six collaboration messages on
  `feat/collab-session-messages`, commit
  `7666e2b100d8cf12998f9b57d3b036964cc3189c` (pushed to `copoka/craft-libs`),
  not in upstream `main` yet. Per ADR-0016 §3 `apps/wordcraft-web/Cargo.toml`
  pins `craft-web-host` to that fork revision
  (`git = "https://github.com/copoka/craft-libs"`), and `Cargo.lock` records the
  same source/rev; it is the only fork dependency — every other craft-libs crate
  stays on its `storytold/craft-libs` revision. The wasm app builds and tests on
  that pin (see §3 and `host-bridge.md` §"Build note"). After the craft-libs PR
  merges, point the URL back at `storytold/craft-libs` and pin the merge commit;
  nothing else in the app changes.
- Web collab has been run with two clients only inside one headless browser
  (two iframes through a cabinet-like host page, §5.1). Two real machines, the
  real cabinet page and its storage, and the visible commit status stay manual.
- A peer that was already connected learns about a newcomer only through
  `presence`, which carries the caret and not the role, so its participant list
  shows `participant` until a role-bearing update arrives (the newcomer itself
  sees every role from the welcome). A join broadcast or a role in presence is
  the follow-up.
- A step of several operations is now one transaction (every inverse or none);
  commands still have no inverse and act as an undo barrier. The `opId` dedup
  window is in memory and is cleared when a room is restored from a snapshot, so
  a retry that crosses a restart is not deduplicated yet.
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

### 5.1 Two-client host scenario (run 2026-11-09)

The acceptance scenario from `TASK.md` and `host-bridge.md`: through the trunk
build, a cabinet-like page hosts two WordCraft clients and drives
`wordcraft:session` → edit → `presence` → `commit` → `wordcraft:committed`.

**Environment.** Chrome 154.0.8037.92 headless (`--use-angle=swiftshader`,
`--enable-unsafe-swiftshader`, device pixel ratio 2), the `dist/web` produced by
`trunk build` (trunk 0.22.1, dev profile, 76.9 MB wasm, 4 m 36 s), and
`wordcraft-collab-server` on `127.0.0.1:8087` with HS256 development tokens
(`aud:"collab"`, `doc:"doc-1"`, `app:"wordcraft"`, one `sid:"session-1"` for both
clients, roles `owner` and `editor`). The host page is served from the same
origin as the app; each pane is `/index.html?host=parent&webgl&author=<name>`
in an iframe, and the page speaks the `craft-web-host` contract: it answers
`ready` with `host` + `session`, records `session-state` / `presence`, and
answers `commit` with `committed`. Input went through the Chrome DevTools
Protocol (mouse and key events), not a person.

**Result.**

| Step | What was observed |
|---|---|
| `wordcraft:ready` | both iframes posted it; the host answered `host` `{version:"1"}` and `session` with their token and role |
| join | `session-state` `synced` in both; the participant list has two ids (`…-2733a7df` owner, `…-2405b064` editor) |
| edit | "Alice was here. " typed in one pane, "Bob was here. " in the other; both screens and the server snapshot read `Bob was here. Alice was here. ` |
| operations | 30 accepted, 0 rejects; snapshot `version=30`; `commits_total` stays 0 (the cabinet owns the file) |
| presence | both apps post `presence` at ~10 Hz; each carries both participant ids; no version moves |
| commit | the owner's Commit button posts `wordcraft:commit {sessionId:"session-1"}` with no `bytes`; the editor has no button; the host answers `wordcraft:committed {version:41, etag:"\"cabinet-etag-41\""}` |
| committed | `HostState` stores the version/etag and shows the status line (unit-tested; see the limitation below on seeing it) |

**A bug this run found (fixed).** Both web clients called themselves
`web-<document_id>`, and a document id is a per-process counter starting at 1:
the two panes both joined as `web-1`. The room keys peers by client id, so the
second join replaced the first peer, each window listed only itself, and the
second client's `(web-1, seq)` operation ids were deduplicated as the first
client's: the text it typed stayed in its own window and never reached the
other. The id now comes from `wordcraft_collab_client::session_client_id`: it
mixes the process id, the clock and a counter natively and a random suffix in
the browser, and is made once per session so reconnects keep their id and
retries stay deduplicated. Call sites: `crates/ui-egui/src/collab.rs` (desktop)
and `apps/wordcraft-web/src/web.rs` (web); test
`two_sessions_on_one_document_get_different_client_ids`.

**What remains for a person.**

1. Two real machines or two real browser profiles over a network, with the real
   cabinet page (its own tokens, iframe policy and `wss://`). This run used one
   headless browser and two same-origin iframes on loopback.
2. The cabinet's own commit: fetch or assemble the file, store it, and return
   the real version and etag. Here the host answered synthetically and, as the
   contract allows when the cabinet owns the file (no `bytes`), stored nothing.
3. The visible confirmation of `committed`: the status line lives for 6 s in the
   status bar and at 1328×771 the Collaboration window covers it; check it on a
   wider window or with the panel collapsed.
4. The human acceptance run: a 100+ page document, then commit and reopen the
   file in Word/LibreOffice.
5. The role shown for a new peer on the side that was already connected (§4).

### 5.2 What is left

1. The `craft-libs` git pin is in place (ADR-0016 §3, `7666e2b1`) and the web
   app builds with trunk on it; the same revision still has to be used by the
   other four apps. The two-client host run is done (§5.1); the manual parts are
   listed there.
2. Mark PR #5834 ready once CI has run and the pin is acceptable to the
   maintainers (GitHub reports `action_required` for fork PRs until a maintainer
   approves).
3. Optional hardening: command inverses for the common engine commands, persisting
   the `opId` dedup window across a snapshot restore, table-cell node ids and
   property primitives, offline queue, the role in presence or a join broadcast.
   (The multi-operation undo transaction is done:
   `a_step_of_several_operations_lands_as_one_transaction`.)
4. Manual acceptance (desktop): two desktops over the real server, a 100+ page
   document, commit and reopen in Word/LibreOffice.

## 6. Where things live

| Concern | File |
|---|---|
| Wire types, journal, room, undo | `crates/collab/src/{wire,journal,room,undo,ids}.rs` |
| Primitives, delta replay, diff | `crates/collab-doc/src/lib.rs` |
| Session commands, remote replay | `crates/collab-engine/src/lib.rs` |
| Client state machine | `crates/collab-client/src/lib.rs` |
| Socket transport | `crates/collab-ws/src/lib.rs` |
| Web session + socket | `crates/collab-web/src/{lib,socket}.rs` |
| Host bridge | `apps/wordcraft-web/src/{host,host_session}.rs` |
| Desktop session | `crates/collab-app/src/lib.rs` |
| UI wiring, panel, carets | `crates/ui-egui/src/collab.rs`, `app/commands.rs` |
| Server | `apps/collab-server/src/{lib,main,store}.rs` |
| Run book | `docs/collab/README.md` |
