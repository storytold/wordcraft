# Real-time collaboration: implementation status

- Date: 2026-11-09
- Branch: `feature/realtime-collaboration` (fork `copoka/wordcraft`)
- PR: <https://github.com/storytold/wordcraft/pull/5834> (draft)
- Base: upstream `main` `4b0d3995`; the branch was rebased onto it (§5.4)
- HEAD at this writing: `cd4c623c` plus this status commit — 55 commits over
  `4b0d3995`; the fork copy still carries the pre-rebase head
  `a6d6ae90` until it is force-pushed
- Protocol: `rfc.md`; server: `server.md`; run book: `README.md`; host bridge:
  `host-bridge.md`.

This page records what is implemented, how it was verified, what is known to be
missing, and where the next session should start.

## 1. What is implemented

### Protocol (`crates/collab`)

- One envelope per operation: `{opId, baseVersion, epoch, kind, target, payload,
  history}`; the server is the single authority (ADR-0001/0003).
- Journal with `opId` dedup, monotonic `version`, `epoch` for structural work,
  tail replay and compaction (ADR-0003/0005). The dedup window is written with
  the snapshot and read back on restore, so a client that retries an operation
  (reconnect with the same `opId`) across a server restart is answered from the
  window and the version does not move (§5.5).
- Session node ids with tombstones and path shifts; a fresh node map is sent when
  an operation moves paths or adds ids (ADR-0002).
- Per-user undo/redo as an operation with a `history` field: one version per undo,
  `undoConflict` when the step's target changed, a step of several operations
  replayed as one all-or-nothing transaction (a refused inverse takes back the
  ones already applied), and commands act as an undo barrier (ADR-0010). The
  exception is a character or paragraph formatting command: the room captures
  the covered paragraphs and stores an exact `set_para_format` inverse, so its
  undo restores the previous value instead of reporting `unsupported` (§5.10).
  An
  operation also reports the nodes beside its target whose content its inverse
  would overwrite (`Effect::touched`): a text edit another user made there after
  the step makes the undo `undo-conflict` instead of dropping that edit (§5.8).
  The accepted answer to an undo/redo carries that `history` action on the wire,
  so the replica that asked applies the inverse instead of discarding its own
  delta (§5.9).
- Role gates: viewer cannot write; commenter may only add/resolve comments
  (including the `review.*Comment` commands); owner alone commits (ADR-0008).
- Presence (ephemeral, never journaled) carries the sender's `role`, stamped by
  the server from the JWT claim, so a peer that was already connected shows a
  newcomer's role on its first frame; the welcome list still gives every role
  to the client that joins.
- Snapshot = model JSON + source `.docx`; `Document::kept` is taken back from the
  source on restore (ADR-0011). Server snapshots every 500 operations, on commit
  and when the last peer leaves; a final snapshot is written on SIGTERM.

### Document adapter (`crates/collab-doc`)

- Primitives: `insert_text`, `delete_range`, `replace_range`, `split_para`,
  `merge_para`, `insert_block` (with the block body and its container path),
  `remove_block`, `move_block` (a run of blocks swapped with the neighbour named
  by its session id, §5.12), `permute` (a contiguous run of paragraphs put in
  the order of an explicit id list, §5.13), `set_para_level` (the outline level
  of a list of paragraphs named by session id, §5.14), `insert_row`, `remove_row`, and
  `set_para_format` (restore one paragraph's runs, mark and `ParaProps` — the
  inverse of a formatting command, §5.10).
- Node ids address every container: a table `[block]`, its rows
  `[block, row]`, its cells `[block, row, cell]` and the paragraphs inside a
  cell `[block, row, cell, block]` each have one (`NodeIds::shift_children`).
  Typing in a cell is an ordinary primitive on that paragraph's id; a row
  insert moves the cell ids with their content, and a removed row tombstones
  its subtree (`NodeIds::remove_subtree`).
- Every accepted operation returns an authoritative delta; a replica applies it
  with `apply_delta` and converges to the same document JSON (tested after each
  operation).
- `diff(before, after, ids)` derives a local command's primitives from the
  document difference, descending into table cells. Cloning a document shares
  its `Arc` blocks, so untouched blocks are one pointer compare. Text becomes
  `replace_range`, Enter/Backspace become `split_para`/`merge_para`, block
  insert/remove and row insert/remove become structural, a block (or a run of
  up to 64) that swapped places with its neighbour becomes `move_block`
  (§5.12), and a wider pure reorder of at most 64 blocks — a sort — becomes one
  `permute` naming the ids in their new order (§5.13). Formatting-only changes,
  a column insert/remove, a cell merge/split and a reorder wider than 64 blocks
  are reported as unsupported (they fall back to a command); a pure reorder the
  primitives cannot name is refused rather than mistaken for a chain of text
  edits. An outline level change leaves the text alone, so it is not a diff
  either: `para.promote`/`demote` travel as a command and the server names the
  levels (§5.14).
- A column removal, a cell merge and a cell split carry an exact inverse
  (`split_cells`/`merge_cells`/`insert_column` with `restore`): the removed
  bodies, the grid and the session ids travel in the inverse, so undo restores
  the table instead of reporting the step as unsupported (§5.8).
- A table command that still arrives as `kind:"command"` (a client that could
  not diff it, or track changes) retires the ids of the tables whose shape it
  changed: their paths mean something else afterwards, so a stale cell id is
  answered `gone` instead of editing another cell.

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
- A formatting command (`format.*` character properties, `para.*` paragraph
  properties, from a named list of property-only commands) captures every
  paragraph between the selection ends before it runs and returns one
  `set_para_format` inverse per paragraph whose formatting changed, carrying
  only the changed fields' previous values; several paragraphs land as one
  transaction. A covered paragraph without a session id is refused before the
  command runs; commands that also move text stay barriers (§5.10).
- A paragraph move (`para.moveUp`/`para.moveDown`, and the `outline.*` pair that
  runs them) is normalized after the command to one `move_block` structural
  operation when the document difference is one adjacent move of at most 64
  blocks; replicas replay the structural delta instead of the command and the
  step has an exact inverse (§5.12). A wider move keeps the command delta and
  its undo answers `unsupported`.
- `para.sort` is normalized after the command to one `permute` structural
  operation over the selected run (at most 64 paragraphs), naming the session
  ids in their new order; replicas replay the same transaction delta and the
  inverse puts the old order back in one step (§5.13). A sort wider than 64
  blocks keeps the command delta and its undo answers `unsupported`.
- The outline level commands (`para.promote`/`para.demote`, the `outline.*`
  pair that runs them, and the Outlining tab's absolute levels) are normalized
  after the command to one `set_para_level` structural operation naming each
  changed paragraph's session id and target level, plus the exact style and
  direct outline level when the level does not imply them; replicas replay the
  same transaction delta, undo puts the earlier levels back in one step and a
  foreign edit in any levelled paragraph makes it `undo-conflict` (§5.14). A
  step wider than 64 paragraphs, or one that also changes a list level, keeps
  the command delta.

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
  history, and the accepted answer's inverse is applied at the author exactly
  once (§5.9).
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
| Per-user undo/redo | done | `undo_and_redo_over_the_socket_change_the_committed_text`, `undo_round_trips_through_the_room`; table inverses §5.8; the author applies its own answer once and a refused undo shows §5.9; character and paragraph formatting commands carry an exact inverse §5.10; a paragraph move and a paragraph sort are addressable structural operations that undo and redo §5.12, §5.13 |
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
CRAFT_I18N_DIR=~/craft-i18n CRAFT_FONTS_DIR=~/craft-fonts cargo xtask ci   # all 9 steps
```

Results at this writing:

- 139 protocol/adapter/client/server tests (138 unit + 1 socket reconnect), all
  green; the LibreOffice commit gate stays the one ignored test. §5.6 added 19:
  4 in `collab`, 13 in `collab-doc` and 2 in `collab-engine` (per-crate counts
  are in §5.6).
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
  wrote `dist/web` (`index.html`, the JS glue, a 76.9 MB dev `_bg.wasm`).- Two-client host run (2026-11-09): Chrome 154 headless (SwiftShader WebGL), the
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
- Rebased onto upstream `main` `4b0d3995` (2026-11-09, §5.4): `cargo xtask ci`
  passed all 9 steps — fmt, clippy (`--workspace --all-targets -D warnings`),
  test, i18n-report, language, doctest, assets, layers (27 crates), wasm (22
  crates); 5775 workspace tests passed, 0 failed, 11 ignored. The run needed
  `CRAFT_I18N_DIR` (`craft-i18n` `1e515422`) **and** `CRAFT_FONTS_DIR`
  (`craft-fonts` `b076417b`): without the fonts three UI tests that draw
  Arabic/CJK interface text fail —
  `panes::tests::navigation_tabs_fit_the_narrowest_pane_in_every_language`,
  `ribbon::tests::ribbon_groups_fit_above_their_labels` and
  `theme::tests::interface_symbols_have_glyphs` — and all three live in files
  unchanged against `main`. `cargo metadata --locked` accepted the rebased lock
  and the CI run did not modify it.
- After the `opId`-window change (§5.5) the affected crates were re-run on the
  build host from the committed tree: `cargo test -p wordcraft-collab` (45
  passed), `cargo test -p wordcraft-collab-server -p wordcraft-collab-client`
  (29 + 13 passed, 1 ignored: the LibreOffice commit gate),
  `cargo clippy -p wordcraft-collab -p wordcraft-collab-server
  -p wordcraft-collab-client --all-targets -- -D warnings` and
  `cargo fmt --all -- --check`, all green.

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
- A step of several operations is now one transaction (every inverse or none);
  character and paragraph formatting commands carry an exact inverse (§5.10),
  including the two that rewrite the characters, `format.changeCase` and
  `format.enclose`, whose inverse restores the paragraph's body (§5.11), and a
  paragraph move, a paragraph sort and an outline level step are addressable
  structural operations with an exact inverse (§5.12, §5.13, §5.14); every
  other command still has none and acts as an undo barrier. The
  `opId` dedup window travels with the snapshot (§5.5), so a retry across a
  restart is
  deduplicated; the window is bounded at 10 000 answers, so a retry after more
  than that (or of an operation accepted after the last snapshot, which the
  restore cannot replay either) is not covered.
- A command that allocates ids (comments, tables, shapes, footnotes…) makes
  replicas request a snapshot after it; there is no id delta on the wire yet.
- Table cells are addressable now, and so are the table's columns and merged
  cells (§5.7): a column insert/remove, a cell merge and a cell split travel as
  structural operations that reindex cell ids instead of retiring them, so the
  caret of a client holding a cell id survives. What still travels as a command
  is a text box, formatting and anything the diff cannot express; a formatting
  command's inverse is a `set_para_format` primitive (§5.10, §5.11), a paragraph
  move is a `move_block` structural operation (§5.12), a sort is a `permute`
  (§5.13) and an outline level change is a `set_para_level` (§5.14), the rest
  stay barriers; a command that
  changes a table's shape retires that table's ids, so a client holding an old
  cell id is refused `gone` and resyncs rather than editing a neighbour. All four
  table edits undo exactly (§5.8): an inserted column, a column removal, a merge
  and a split each carry the data their inverse needs, and the inverse refuses
  (`undo-conflict`) when the table moved on. The node map is larger: it carries
  every row, cell and cell paragraph, so a document with a 2000-cell table sends
  ~3× more `nodes` entries than before; the server bounds the document at 64 MiB
  and the map is sent on join and after structural operations.
- An inverse carries the bodies it puts back, so it is bounded: a merge, a split
  or a column removal whose restore data would pass 256 KiB keeps no inverse and
  undo reports that step as `unsupported` (§5.8). A formatting step carries the
  previous runs, mark and paragraph properties of the paragraphs it changed —
  and, for a case change or an enclosure, their text and inline objects — and is
  bounded at 256 paragraphs, so a formatting action over a wider selection stays
  a barrier too (§5.10, §5.11). A structural step still cannot
  be undone once any later structural operation landed, so two merges in a row
  can be undone only newest first (§4, ADR-0010).
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
5. The role shown for a new peer on the side that was already connected: the
   bug is fixed and covered by tests (§5.3); the human run still has to see it
   in the panel.

### 5.2 What is left

1. The `craft-libs` git pin is in place (ADR-0016 §3, `7666e2b1`) and the web
   app builds with trunk on it; the same revision still has to be used by the
   other four apps. The two-client host run is done (§5.1); the manual parts are
   listed there.
2. Force-push the rebased branch (§5.4), then mark PR #5834 ready once CI has run
   and the pin is acceptable to the maintainers (GitHub reports
   `action_required` for fork PRs until a maintainer approves, and the PR shows
   `CONFLICTING` until the force-push).
3. Optional hardening: property
   primitives, an offline queue, or a join broadcast that carries the role before
   the first presence frame (presence already carries it, §5.3). (The
   multi-operation undo transaction is done:
   `a_step_of_several_operations_lands_as_one_transaction`; the `opId` window
   survives a snapshot restore: §5.5; table-cell ids are done: §5.6; columns,
   merges and splits are primitives: §5.7; their inverses are done and an undo
   answer lands at its author: §5.8, §5.9; command inverses for the frequent
   character and paragraph formatting commands are done: §5.10; a paragraph
   move is an addressable structural operation that undoes: §5.12.)
4. Manual acceptance (desktop): two desktops over the real server, a 100+ page
   document, commit and reopen in Word/LibreOffice.

### 5.3 Role in presence (run 2026-11-09)

§5.1 left one visible gap: a peer that was already connected learned about a
newcomer only from `presence`, which carried the caret and not the role, so its
participant list showed `participant` until the next welcome. `TASK.md` §3 asks
for "a participant list with roles", so the role now travels with presence.

- **Wire.** `ServerMsg::Presence` gains `role` (`#[serde(default)]`, so a frame
  from an older server still parses). The client's `Presence` message is
  unchanged: a client cannot name a role, the server stamps it from the JWT
  claim (ADR-0008), the same authority that gates write and commit.
- **Server.** `process_frame` copies the authenticated role into every presence
  broadcast. The room already keeps it per peer (`Peer.role`) for the welcome
  list and the lease checks; nothing new is trusted from the wire.
- **Client.** `Event::Presence` carries the role. `collab-app` folds a frame
  into the participant list through `apply_presence` (shared by the web session
  in `collab-web`): a known peer gets the role updated in place, a new peer is
  added with it, and an empty role (older server) keeps the role already known
  instead of overwriting it with `participant`.
- **UI.** Nothing else was needed: the desktop Collaboration panel already
  renders `id · role` from `ParticipantView`, and the host bridge already sends
  `hostParticipants` with `role` to the cabinet page.

- **Docs.** `server.md` and `rfc.md` now spell the control-message fields as the
  wire does (`client_id`, `session_id`, `expected_version`, …): serde's
  `rename_all` on an enum renames variants, not their fields, so the envelope
  structs stay camelCase while the message fields are snake_case. The presence
  row carries `client_id` and the server-added `role`.

**Test.** `presence_carries_the_sender_role_to_peers_that_joined_earlier`
(`apps/collab-server/src/tests.rs`) connects three real WebSocket clients —
`owner`, `editor`, `viewer` — checks that each welcome lists the roles already
present, then has each client publish presence and asserts that both peers that
joined earlier receive the sender's raw role (`viewer`, `editor`, `owner`).
`presence_carries_the_sender_role` (client) and the `apply_presence` tests
(collab-app: new peer, placeholder replacement, role-less frame) cover the two
halves without a socket; `an_older_presence_without_a_role_still_parses` (wire)
pins the frame of a server built before the field.

### 5.4 Rebase onto upstream `main` `4b0d3995` (run 2026-11-09)

GitHub reported PR #5834 as `CONFLICTING`: `main` had moved from the branch's
merge base `2a0a20ae` to `4b0d3995` (279 files, a new page command API among
them). The branch was rebased in place, no commit dropped:

| | |
|---|---|
| before | `a6d6ae90` — 32 commits over `2a0a20ae` |
| after | `d39ef64c` — the same 32 commits over `4b0d3995` |
| backup ref | `backup/pre-rebase-a6d6ae90` (local) |
| push | not made — the fork needs a manual `--force-with-lease` |

**Conflicts and how they were resolved.**

1. `apps/wordcraft-web/src/web.rs` — `main` added the page command API
   (`crate::api::install` + `WordApp::with_control`, craft-libs#413) where this
   branch builds the app and installs `host_commit`. Both are kept: the control
   channel is installed first, the commit bridge right after it.
2. `apps/wordcraft-web/Cargo.toml` — `main` now pins `craft-web-api` and
   `craft-web-host` at `storytold/craft-libs@fd5e2fe`; this branch moves
   `craft-web-host` to `copoka/craft-libs@7666e2b1` (ADR-0016 §3).
   `craft-web-api` stays on the storytold revision: the fork revision's
   `craft-web-host` does not depend on it (its lock entry lists only
   `wasm-bindgen` and `web-sys`), so the two sources do not yield a second copy
   of one crate. `git diff 7666e2b1 fd5e2fe -- crates/web-api` is empty as well,
   so the API crate is the same code on either revision.
3. `Cargo.lock` — auto-merged everywhere except the `craft-web-host` source
   line, which now reads
   `git+https://github.com/copoka/craft-libs?rev=7666e2b1…#7666e2b1…`. The lock
   was not regenerated by hand: `cargo metadata --locked` on the build host
   accepted it, and after the whole `cargo xtask ci` run the lock is identical
   to the committed one (`git status --short` and `git diff --stat Cargo.lock`
   are empty on the build host).

**Gates on the rebased tree.** Build host `agents`, `scripts/remote-build.sh`:

- `cargo metadata --locked` → `LOCK_OK`, lock unchanged.
- `cargo xtask ci` → all 9 steps passed (fmt, clippy, test, i18n-report,
  language, doctest, assets, layers, wasm); 5775 workspace tests passed, 0
  failed, 11 ignored. Details and the two environment-only failure modes that
  need `CRAFT_I18N_DIR`/`CRAFT_FONTS_DIR`: §3.

**Still to do by hand:** force-push the rebased branch to `copoka/wordcraft`
(`git push --force-with-lease fork feature/realtime-collaboration`); the PR page
stays `CONFLICTING` until then.

### 5.5 The `opId` window survives a snapshot restore (run 2026-11-09)

§4 recorded the gap: the dedup window lived in `Journal` memory only and
`Journal::restore` cleared it, so after the server came back from a snapshot a
client retrying an operation it never saw acknowledged (a reconnect resends the
unconfirmed queue with the same `opId`) had that operation applied twice.

**What is stored.** The window is server-side state, so nothing about it goes on
the wire and the protocol is unchanged — a retry is still an ordinary
`clientMessage{type:"op"}` with the same `opId`, and the answer is still
`applied{...}`.

- `crates/collab/src/journal.rs`: `DedupWindow` (`{entries: [Applied…]}`, oldest
  first) plus `Journal::dedup()` and `Journal::restore_with(version, epoch,
  window)`. A restored window is clamped to answers at or before the restored
  head (an answer past the head would hide work the snapshot lost) and to the
  journal's bound (10 000), dropping the oldest first, exactly as a live journal
  evicts.
- `crates/collab/src/room.rs`: `Room::restore(model, version, epoch, dedup)` and
  `Room::dedup()`; `submit` already checks the window before `journal.check`, so
  the restored room answers a repeat with the stored `Applied` (delta included)
  and neither applies the operation nor bumps the version.
- `apps/collab-server/src/store.rs`: `Snapshot` gains `dedup`; it is written to
  `dedup.json` next to `model.json`/`meta`/`source.docx`, rotated with the
  previous snapshot, and written before `meta` (the commit marker). A snapshot
  without `dedup.json` (written before this change) or with an unreadable one
  loads with an empty window: the document still opens, and dedup across the
  restart is the only thing missing.
- `apps/collab-server/src/lib.rs`: `persist` reads `room.dedup()` under the same
  lock as the version and the model, so the window always matches the snapshot's
  version; `room_for` passes `snapshot.dedup` to `Room::restore`.

**Tests.**

- `the_dedup_window_survives_a_restore`, `a_restore_drops_answers_newer_than_its_head`,
  `a_restored_window_respects_its_bound` (`crates/collab/src/journal.rs`).
- `a_repeat_after_a_snapshot_restore_is_not_applied` (room, fake model),
  `a_restore_without_a_window_still_starts_clean` (`crates/collab/src/tests.rs`).
- `a_retry_after_a_snapshot_restore_is_not_applied_twice`
  (`apps/collab-server/src/tests.rs`): a real `AppState` and data directory — one
  accepted `insert_text`, `persist`, drop the state, a new `AppState` on the same
  directory — then the same `opId` again: the stored `Applied` comes back, the
  version stays 1 and the text stays `Hi`.
- `the_op_id_window_round_trips_with_the_snapshot`, `a_snapshot_without_a_window_still_opens`,
  `a_corrupt_window_does_not_lose_the_document` (`apps/collab-server/src/store.rs`).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab` 45 passed; `cargo test -p wordcraft-collab-server -p
wordcraft-collab-client` 29 passed + 13 passed, 1 ignored (the LibreOffice commit
gate, needs `soffice`); `cargo clippy -p wordcraft-collab -p wordcraft-collab-server
-p wordcraft-collab-client --all-targets -- -D warnings` clean; `cargo fmt --all
-- --check` clean.

- After the table-cell ids (§5.6) the affected crates were re-run on the build
  host from the committed tree: `cargo test -p wordcraft-collab
  -p wordcraft-collab-doc -p wordcraft-collab-engine -p wordcraft-collab-app
  -p wordcraft-collab-client` (49 + 32 + 15 + 10 + 13 passed),
  `cargo test -p wordcraft-collab-server -p wordcraft-collab-ws
  -p wordcraft-collab-web` (29 passed + 1 ignored, the LibreOffice gate; 1 socket
  test; 5 web tests), `cargo clippy` on the same crates with
  `--all-targets -- -D warnings` and `cargo fmt --all -- --check`, all green.

### 5.6 Table cell ids (run 2026-11-09)

`TASK.md` §8 lists a structural conflict in a table (insert or delete of a row)
as a test that must not lose data; §4 recorded that a table cell had no id, so
every edit in a cell travelled as a command and could make the room resync. The
node map now addresses the whole document tree.

- **Ids.** `NodeIds` keys are the full `doc::Path`, and `DocModel::index_nodes`
  walks every container: a table `[block]`, each row `[block, row]`, each cell
  `[block, row, cell]` and every block inside a cell (nested tables included).
  `NodeIds::shift_children(story, container, at, delta)` moves the child index
  in one container and leaves other containers alone; `NodeIds::remove_subtree`
  tombstones a removed block, row or cell with everything under it, so a late
  operation is `gone` instead of landing on a neighbour. `shift_top_blocks` is
  now the empty-container case of `shift_children`.
- **Primitives.** `split_para`/`merge_para` shift their own container
  (`path.parent()`), not the top-level block index — before, splitting a
  paragraph in a cell shifted the story's blocks. `insert_block`/`remove_block`
  take an optional `path` (the container) next to `index`; with no index they use
  the target node's own container and position. Two structural operations are
  new: `insert_row {index, row}` and `remove_row {index}`, addressed at the table
  or at any node inside it; the row body travels in the operation and in its
  inverse, so undo and replicas never have to guess which neighbour a row was
  copied from.
- **Diff.** `collab-doc::diff` descends into cells: a paragraph inside a cell
  becomes `replace_range` on its id, a changed table is compared row by row and
  cell by cell (`same_row`/`same_cell` use the shared `Arc` blocks), a row that
  appears or disappears becomes `insert_row`/`remove_row`. A column insert, a
  merge/split and a formatting-only change are still `Unsupported` and fall back
  to `kind:"command"`.
- **Commands.** `SessionModel::apply_command` compares the table shape (rows and
  cells per row) for `table.*`/`insert.*` commands; when it changed, the ids of
  the moved tables are tombstoned and reissued and `nodes changed` is reported.
  A client that could not normalize the command and still holds an old cell id
  is refused `gone` and resyncs — it never edits another cell.

**Tests.**

- `crates/collab/src/ids.rs`: `a_row_inserted_above_moves_the_cells_it_does_not_touch`,
  `a_row_removed_takes_its_cells_and_the_later_rows_move_up`,
  `removing_a_table_block_removes_every_node_below_it`,
  `a_shift_inside_a_cell_leaves_the_table_and_its_other_cells_alone`.
- `crates/collab-doc/src/tests.rs`: `every_table_node_gets_an_id_of_its_own`,
  `two_replicas_edit_different_cells_of_one_table_and_converge` (two replicas,
  two cells, the same JSON), `a_repeated_cell_edit_after_a_reconnect_is_applied_once`
  (one version, one insertion),
  `a_row_insert_moves_the_cell_ids_and_an_active_edit_lands_in_its_own_cell`,
  `a_removed_row_tombstones_its_cells_and_a_late_edit_is_gone`,
  `splitting_a_paragraph_inside_a_cell_moves_only_that_cell`,
  `typing_in_a_cell_diffs_to_a_replace_range`,
  `inserting_a_row_diffs_to_insert_row`, `deleting_a_row_diffs_to_remove_row`,
  `inserting_a_table_diffs_to_insert_block_and_its_cells_get_ids`,
  `a_column_insert_is_still_reported_as_unsupported`,
  `undo_of_a_row_insert_removes_the_row_again`,
  `a_replica_applies_a_row_insert_and_a_row_removal`.
- `crates/collab-engine/src/tests.rs`:
  `a_table_row_insert_lands_through_the_session_adapter_and_moves_cell_ids`,
  `a_table_command_retires_the_cell_ids_it_moves`.

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app -p wordcraft-collab-client` — 49, 32, 15, 10, 13 passed;
`cargo test -p wordcraft-collab-server -p wordcraft-collab-ws -p
wordcraft-collab-web` — 29 passed + 1 ignored (LibreOffice gate), 1 socket test,
5 web tests; `cargo clippy` on those crates with `--all-targets -- -D warnings`
clean; `cargo fmt --all -- --check` clean. No push was made and CI is unchanged.

### 5.7 Columns, merges and splits as addressable primitives (run 2026-11-09)

`TASK.md` §8 lists a structural conflict in a table as a test; §5.6 made rows
and cells addressable but left a column insert/remove, a cell merge and a cell
split as commands. Each of those changes a table's shape without adding or
removing a row, so every replica resynced and a client holding a cell id was
refused `gone` — the client's caret was lost. Four structural operations now
cover them: **head `9c035abc`** on `feature/realtime-collaboration`, with this
entry's docs commit on top (no push, CI unchanged).

**The operations.** All four address a table by its node id (the table or any
node inside it) and carry a grid address, not cell ids, so a client does not
have to guess what `Table::insert_col`/`merge`/`split_cell` will do:

- `insert_column {index}` / `remove_column {index}` — `index` is the grid column.
- `merge_cells {rows: [r0, r1], cols: [g0, g1]}` — a rectangle of grid columns.
- `split_cells {row, cell, columns}` — one cell cut into `columns` cells.

The authoritative delta repeats the operation with the table's `nodeId`, so a
replica recomputes the same plan from the same table; the row operations already
worked that way (§5.6).

**Ids are reindexed, not retired.**

- A row whose cells do not span the inserted or removed column shifts its cell
  ids after the change by one with their whole subtree
  (`NodeIds::shift_children`): a client editing such a cell keeps its id, and an
  edit planned before the insert still lands in its own cell. A cell that spans
  the column only widens or shrinks and keeps its id.
- `merge_cells` keeps the id of the first cell of each merged row (and the ids
  of the blocks that stayed at the same place in it), tombstones the cells it
  absorbs, and reissues the ids inside a cell whose block list it rewrote (a
  continued row is drained into the cell above). `NodeIds::remove_descendants`
  is new: it retires the blocks under a node and keeps the node's own id, which
  is what a drained cell needs.
- `split_cells` keeps the id and the content of the cell it cuts and issues ids
  for the new parts, so the caret holder keeps typing in the first part.
- A row that would lose its only cell to `remove_column` disappears from the
  table (the document's `delete_col` drops it) and the rows below move up; the
  map follows rather than leaving an id on the wrong row.

**Refused before the document changes.** A merge that is past the last row or
column, inverted, a single cell, or that starts or ends inside a vertical merge
cannot be addressed cell by cell; a split with fewer than two parts or one the
grid cannot take (`Table::split_cell` returns the cell unchanged) is refused
too. Each returns `rejected{code:"invalid"}` with the table untouched and the
room version unchanged — a test asserts the document JSON is byte-identical
after five such operations.

**Diff.** `collab-doc::diff` recognizes the four changes by replaying the
document's own table primitive on a copy of the old table and comparing it with
the new one — the probe that reproduces the shape names the operation and its
address. A column insert/remove is found among the grid indexes, a merge among
the cell boundaries of the first changed row, a split among the cells of the one
row that gained cells (a split can widen cells of the other rows, which the
comparison sees). Anything else still falls back to `kind:"command"`.

**Undo.** An inserted column carries `remove_column` as its inverse (exact: the
column did not exist before). A column removal, a merge and a split have no
inverse — the removed bodies and the refined grid are not carried — so
`Effect::inverse` is empty and the step is `unsupported`, as a command is
(ADR-0010). That is not a regression: all three were commands with no inverse
before.

**Tests.**

- `crates/collab/src/ids.rs` (unchanged suite) plus
  `remove_descendants` exercised through the merge tests below.
- `crates/collab-doc/src/tests.rs`:
  `inserting_a_column_diffs_to_insert_column`,
  `removing_a_column_diffs_to_remove_column`,
  `merging_cells_diffs_to_merge_cells`, `splitting_a_cell_diffs_to_split_cells`,
  `a_column_insert_matches_the_documents_own_insert_col` (a merged cell makes
  one row widen and the others gain a cell; the adapter's plan must be the one
  `Table::insert_col` follows),
  `a_column_insert_moves_the_cell_ids_and_the_caret_holder_keeps_his` (an edit
  planned before the insert lands in its own cell after it),
  `two_replicas_insert_a_column_and_converge`,
  `removing_a_column_keeps_the_ids_of_the_cells_it_does_not_touch`,
  `removing_a_column_through_a_replica_matches_the_room`,
  `merging_two_cells_keeps_the_first_cells_id_and_a_split_brings_the_cells_back`
  (room + replica: converge after the merge, the absorbed cell is `gone`, the
  caret paragraph keeps its id through the split and still types),
  `a_merge_range_that_is_not_addressable_is_refused_before_the_document_changes`,
  `undo_of_a_column_insert_removes_the_column_again` (the inserted column is the
  inverse and restores the table exactly).
- `crates/collab-engine/src/tests.rs`:
  `a_column_insert_lands_through_the_session_adapter_and_keeps_the_caret_cell`,
  `a_merge_lands_through_the_session_adapter_and_the_caret_cell_keeps_its_id`.
- `a_column_insert_is_still_reported_as_unsupported`, which pinned the old
  limitation, is gone; the four `*_diffs_to_*` tests replace it.

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app -p wordcraft-collab-client` — 49, 43, 17, 10, 13 passed;
`cargo test -p wordcraft-collab-server -p wordcraft-collab-ws -p
wordcraft-collab-web` — 29 passed + 1 ignored (the LibreOffice commit gate), 1
socket test, 5 web tests; `cargo clippy` on both groups with `--all-targets --
-D warnings` clean; `cargo fmt --all -- --check` clean. No push was made and CI
is unchanged.

### 5.8 Inverses for a merge, a split and a column removal (run 2026-11-09)

§4 recorded that the three table edits have no inverse, so per-user undo reported
them as `unsupported`; §5.7 left them as the last undo gap in a table. Each now
captures what it takes away *before* it changes the document and ships that in
its inverse operation: **head `79019d58`** on `feature/realtime-collaboration`,
with this entry's docs commit on top (no push, CI unchanged).

**What each inverse carries.** The inverse is the same structural operation with
a `restore` object; the forward operation's delta is unchanged, so a client that
did not understand the inverse still applies the forward edit.

- Merge → `split_cells` with `restore`: per row, the cells the range absorbed
  (`props` and `blocks`), and the session ids of everything below them. The
  inverse replaces the merged cell of each row with the row's original cells,
  moves the cells after it right again and puts the ids back — so the cell a
  merge absorbed is addressable under its old id again, and the caret holder
  keeps typing where he was.
- Split → `merge_cells` with `restore`: the grid before the cut, the cell it cut,
  how many parts it added, and the props of the cells of the other rows that
  widened over the refined grid. The inverse merges the parts back, restores the
  grid (the line the split added is gone) and retires the parts' ids.
- Column removal → `insert_column` with `restore`: the grid width, the body of
  every cell that vanished, the props of the cells that only shrank, the rows
  that disappeared with the column, and their ids. The inverse re-inserts the
  grid column, puts the cells and rows back at their indexes, shifts the later
  ones right again and restores the ids.

**Refused before anything changes.** A restore validates the payload and compares
the table's shape (the spans of every row's cells, captured after the forward
edit) with the table it is about to edit *before* it touches the document or the
id map. A mismatch answers `undo-conflict` with the document unchanged; a
malformed payload answers `invalid`. The forward edits keep refusing an
unaddressable range before mutating (§5.7).

**No silent loss on undo.** An operation now reports the nodes beside its target
whose content its inverse would overwrite (`Effect::touched`): the merged cells
of a merge, the cell and its parts of a split, the cells an insert created (and
the block or row an insert added). The room keeps the version that last touched
each of them and refuses a step whose touched ids moved on, so a text edit
another user made inside a merged cell, a split part, an inserted row or an
inserted column answers `undo-conflict` instead of vanishing with the undo. An
edit in a cell the step did not touch does not block it (tested).

**Bounds.** An inverse's restore data is capped at 256 KiB and 16 384 ids: a
region too large to carry keeps no inverse and undo reports the step as
`unsupported`, as before. A structural step still cannot be undone once any later
structural operation landed (ADR-0010), so two merges in a row undo newest first.

**Tests.**

- `crates/collab-doc/src/tests.rs`:
  `undo_of_a_merge_splits_the_cells_back_with_their_own_text_and_ids` (merge →
  undo → the cells, their text and their ids come back; the caret holder types in
  the absorbed cell; the replica converges),
  `undo_of_a_split_merges_the_parts_back_and_restores_the_grid`,
  `undo_of_a_column_removal_puts_the_column_back_with_its_text_and_ids`,
  `undo_of_a_column_removal_restores_a_row_that_disappeared_with_the_column`,
  `undo_of_a_merge_after_another_users_edit_in_the_cell_is_a_conflict` (the
  document is byte-identical after the refusal, the version does not move, the
  step stays on the stack, the other user's text survives),
  `undo_of_a_merge_lands_after_an_edit_in_another_cell` (the touched set is the
  region, not the table),
  `undo_of_a_row_insert_after_an_edit_in_the_new_row_is_a_conflict`.
  Three of them drive two replicas (merge, split and column removal): the room
  applies the undo and a replica applies its delta to the same JSON.
- `crates/collab/src/{room,undo}.rs`: `Effect::touched` and `UndoStep::touched`
  (serde-default, so a step serialized before the field still loads) are covered
  through the doc tests above; the undo tests there are unchanged.

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine` — 49, 50, 17
passed; the wider run `-p wordcraft-collab-client -p wordcraft-collab-app -p
wordcraft-collab-ws` — 13, 10 and 1 (the socket test) passed; `cargo test -p
wordcraft-collab-server -p wordcraft-collab-web` — 29 passed + 1 ignored (the
LibreOffice commit gate) and 5 passed; `cargo clippy` on the three crates with
`--all-targets -- -D warnings` clean; `cargo fmt --all -- --check` clean. No push
was made and CI is unchanged.

### 5.9 The author of an undo applies its own inverse (run 2026-11-09)

§4 recorded that an undo reached every peer but not the replica that asked for
it: the room applied the inverse, but `SyncState::apply` skipped every answer
carrying the caller's own `opId`, and a `history` operation is never applied
optimistically. **Head `eda7924f`** on `feature/realtime-collaboration`, with
this entry's docs commit on top (no push, CI unchanged).

**What the answer now says.** `Applied` gains `history` (serde-default, so the
stored dedup window and older snapshots still load); `Journal::accept` fills it
from `op.history`, so a deduplicated retry carries it too. `SyncState::apply`
skips an own delta only when that field is absent. An own history answer is
replayed like a remote delta — it is the only copy of the inverse, because the
engine never ran for it — and lands exactly once; the ordinary optimistic edit
keeps its skip (`our_own_answer_is_not_applied_twice`). The delta is replayed by
the same `apply_delta` for text and for the table inverses: a merge, a split and
a column removal undo through this path unchanged, and a `transaction` answer
lands all-or-nothing on a scratch copy.

**A refusal is shown, not swallowed.** `refusal_status` (shared by the desktop
and web sessions) keeps `undo-conflict` and `unsupported` in `InSync` with the
step still on the stack — the room refused before changing anything — and shows
the reason in `last_error`; `stale`, `gone`, an expired lease or token move to
`Resyncing` and ask for a snapshot (the desktop now requests one for a retryable
refusal as well, as the web session already did). The error clears when the
snapshot lands.

**Tests.**

- `crates/collab-app/src/tests.rs`:
  `the_author_and_a_peer_apply_an_undo_answer_once` (two edits, then undo →
  redo → undo → redo: the author sees each state without a snapshot, the peer
  converges, and a later edit on the same node still refuses the older step),
  `a_table_undo_reaches_its_author_and_the_peer_like_a_text_undo` (merge, split
  and remove_column each apply the forward delta at the author, then undo back
  to the original table JSON on both replicas),
  `an_undo_in_a_table_is_refused_at_the_author_without_losing_the_peers_edit`
  (the room answers `UndoConflict`, the author's document is byte-identical
  before and after the refusal, the peer's text and the room version survive),
  `a_refused_undo_is_an_error_and_a_stale_answer_resyncs` (status mapping).
- The `Applied` literals in `crates/collab-client`, `crates/collab-ws` and
  `apps/collab-server` gained the default field; their tests are unchanged.

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 49, 50, 17 and 14 passed; the wider run `-p
wordcraft-collab-client -p wordcraft-collab-ws -p wordcraft-collab-server -p
wordcraft-collab-web` — 13, 1 (the socket test), 29 + 1 ignored and 5 passed;
`cargo clippy` on those six crates with `--all-targets -- -D warnings` clean;
`cargo fmt --all -- --check` clean. No push was made and CI is unchanged.

### 5.10 Inverses for character and paragraph formatting (run 2026-11-09)

§5.2 п.3 listed "command inverses for the common engine commands" as optional
hardening, and §4 recorded the cost: every `kind:"command"` step was an undo
barrier, because the room replays a command at every replica but has no way to
replay it backwards — a toggle would flip again and a relative step (grow font)
would move twice. **Head `2ce54064`** (`0ad0d2ef` adds the addressability guard) on
`feature/realtime-collaboration`, with
this entry's docs commit on top (no push, CI unchanged).

**The forward effect is unchanged.** A formatting command still travels as
`kind:"command"` and every replica replays it (`apply_delta` →
`Session::run_remote`); no wire field changed. Only the step now carries an
inverse: `crates/collab-doc` gained the primitive `set_para_format`, which
restores one paragraph's `runs`, its `mark` (the formatting of the paragraph
mark) and its paragraph `props`. Each field is optional, so the inverse carries
only what the command changed, with the values captured **before** the command
ran.

**How the server builds it.** `SessionModel::apply_command` captures every
paragraph between the ordered selection ends before running a command from
`CHAR_FORMAT_COMMANDS`/`PARA_FORMAT_COMMANDS` (`crates/collab-engine`). That
range is a superset of what the command touches: a selection, the word under a
collapsed caret and the mark of an empty paragraph all live in those paragraphs.
After the run it compares text length, runs, mark and `ParaProps` per paragraph
and emits one `set_para_format` per paragraph whose formatting changed. Several
paragraphs become several inverse operations and therefore one
`{"op":"transaction","ops":[…]}` delta, as a table step already does. Redo still
replays the original command — with the exact previous value back in place, a
toggle or a relative step computes the same result it did the first time.

**Addressable, or refused before anything changes.** If a paragraph the command
could touch has no session id, the inverse could not name it: the command is
refused as `invalid` before `Session::run`. The primitive itself refuses a node
that is not a paragraph (a table, a row, a cell, an unknown or a deleted id),
a run list that does not cover the current text byte for byte, a run that ends
inside a character, an empty patch, and more than `MAX_FORMAT_RUNS` (100 000)
runs — all before the document is touched, with the patch parsed from a clone.

**A foreign edit of the same fields is `undo-conflict`.** The step reports every
changed paragraph in `Effect::touched`. Any later operation by another user in
one of those paragraphs — typing or formatting — makes undo answer
`undo-conflict` without changing the document at any replica and without moving
the room version; the step stays on the stack and the session stays `InSync`
(§5.9). An edit in a paragraph the step did not touch does not block the undo.

**What stays a barrier.** Only commands a paragraph snapshot can name are
listed; §5.11 lifts the two that rewrite text out of this list, §5.12 lifts the
adjacent paragraph move, §5.13 lifts the paragraph sort and §5.14 lifts the
outline level commands (`para.promote`/`demote`). Commands that move text past
more than 64 blocks
(`para.moveUp`/`moveDown` over a wider selection), strip text
(`para.removeBlank`, `para.convertBreaks`, the space/indent text tools),
allocate numbering definitions (`para.bullets`, `para.numbering`,
`para.multilevel`, `para.restartNumbering`) or otherwise
change more than `text`/`objects`/`runs`/`mark`/`props` still have no inverse,
and their undo answers `unsupported` — as do a step over more than 256
paragraphs, and a collapsed-caret formatting command that only sets the
session's pending formatting (no document change to restore).

**Tests.**

- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s): `a_char_format_undo_returns_the_previous_value_at_its_author_and_the_peer`
  (bold as a planned command, undo → the exact earlier document JSON at the
  author and the peer, redo → bold again),
  `a_foreign_format_change_refuses_the_undo_without_touching_the_document`
  (Bob italicises the same paragraph; Alice's undo is `undo-conflict`, both
  documents byte-identical before and after, room version unchanged),
  `a_two_paragraph_align_undo_lands_as_one_transaction_on_both_replicas`
  (`para.alignCenter` over two paragraphs → one `transaction` delta restoring
  `ParaProps` at both replicas).
- `crates/collab-engine/src/tests.rs`:
  `a_formatting_command_carries_the_previous_value_as_its_inverse` (undo back to
  the exact JSON, then redo),
  `a_paragraph_align_undo_restores_every_covered_paragraph`,
  `a_command_that_moves_text_is_still_an_undo_barrier` (`format.changeCase` →
  `unsupported`, text unchanged),
  `a_format_restore_on_a_node_that_is_not_a_paragraph_is_refused_before_mutation`
  (a table id → `invalid`, document and version unchanged),
  `a_format_restore_that_does_not_cover_the_text_is_refused_before_mutation`
  (999-byte and 2-of-5-byte run lists → `invalid`, nothing written),
  `a_formatting_command_over_a_paragraph_without_a_session_id_is_refused_before_mutation`
  (a covered paragraph whose id the room retired → `invalid`, document and
  version unchanged).
- `crates/collab-doc/src/tests.rs`:
  `a_format_restore_rewrites_the_runs_and_properties_it_names` (the adapter's
  own round trip: apply, then the stored inverse restores the exact JSON),
  `a_format_restore_on_an_unknown_node_is_refused_before_mutation`.

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app -p wordcraft-collab-client -p wordcraft-collab-ws` — 49,
52, 23, 17, 13 and 0 plus 1 socket test, all passed; `cargo clippy` on those six
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.11 The body a text-rewriting command changed comes back with it (run 2026-11-09)

§5.10 left `format.changeCase` and `format.enclose` with `text` as undo
barriers: a formatting-only inverse would put the old `runs` over the new text.
**Head `79940cc6`** on `feature/realtime-collaboration`, with this entry's docs
commit on top (no push, CI unchanged).

**The snapshot now includes the body.** A formatting command is still captured
as every paragraph between the ordered selection ends, but `FormatSnapshot`
(`crates/collab-engine`) now also keeps the paragraph's `text` and inline
`objects` beside `runs`, `mark` and `props`. After the command it compares the
state of each paragraph: a paragraph whose characters changed gets a `set_para_format`
that carries the previous `text`, the previous `objects` (an enclosed-character
field lives there), the `runs` of that text, and `expect` — the text the
paragraph holds **after** the command; `mark` and `props` travel only if they
changed too. A case change keeps each run's formatting, and the inverse puts
the exact earlier text/run composition back; un-enclosing removes the field and
restores the plain characters.

**The primitive validates the body before it mutates.** `set_para_format`
(`crates/collab-doc`) accepts `text`, `objects` and `expect`. All checks run
before the document is touched: the address must name a paragraph (a table, a
row, a cell, an unknown id → `invalid`; an id the room retired → `gone`);
`expect` must equal the paragraph's current text; `objects` must match the
number of `U+FFFC` characters in the target text; the runs must cover the target
text byte for byte and end on character boundaries; and a body restore must
carry both its `objects` and its `runs`. The adapter keeps the previous body and
objects as its own inverse, so a rollback of a multi-operation step restores
them too.

**One transaction, the same JSON at every replica.** Several changed paragraphs
become several `set_para_format` operations and therefore one
`{"op":"transaction","ops":[…]}` delta, as a table step already did.
`apply_delta` replays that delta on the author's and the peer's `Session` from
the same JSON, so both end at the exact document JSON the room holds; redo still
replays the original command over the restored text.

**A foreign edit of any captured paragraph is `undo-conflict`.** The step
reports every captured paragraph in `Effect::touched`. Any later operation by
another user in one of them — typing, formatting or another rewrite — makes
undo answer `undo-conflict` without changing the document at any replica and
without moving the room version; the step stays on the stack and the session
stays `InSync` (§5.9).

**What stays a barrier.** A step over more than 256 paragraphs is not captured
at all: the command still runs, but the step has no inverse and its undo answers
`unsupported` instead of sending an unbounded transaction. Commands that move or
reorder blocks in other ways, allocate numbering definitions or change more than
a paragraph's
body and formatting are unchanged from §5.10 (an adjacent paragraph move is
lifted in §5.12). The client's diff fallback is
unchanged too: a case change planned from the document difference still travels
as a `replace_range` primitive (whose undo is a text edit); this entry covers
the command path — track changes on, the CLI/MCP, or any client that submits the
command itself.

**Tests.**

- `crates/collab-engine/src/tests.rs`:
  `a_case_change_undo_restores_the_previous_text_and_runs` (two differently
  formatted runs, undo → the exact earlier JSON, delta is one
  `set_para_format` naming `text`/`expect`, redo → upper again),
  `an_enclose_undo_removes_the_field_and_restores_the_text` (one inline object
  after the command, undo → the exact earlier JSON, `objects: []`),
  `an_undo_delta_replays_on_a_replica_session` (a second `Session` learns the
  command and the undo only from deltas and ends at the exact earlier document),
  `a_step_over_more_than_256_paragraphs_stays_an_undo_barrier` (300 paragraphs:
  the command lands, undo → `unsupported`, text unchanged),
  `a_body_restore_on_a_retired_node_is_refused_before_mutation` (retired id →
  `gone`, document and version unchanged).
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s):
  `a_two_paragraph_case_change_undo_lands_as_one_transaction_on_both_replicas`
  (one `transaction` delta with two body restores, author and peer at the exact
  earlier JSON),
  `an_enclose_undo_removes_the_field_at_its_author_and_the_peer`,
  `a_foreign_edit_of_a_rewritten_paragraph_refuses_the_undo_without_touching_the_document`
  (Bob types into the paragraph, Alice's undo is `undo-conflict`, both documents
  byte-identical before and after, room version unchanged).
- `crates/collab-doc/src/tests.rs`:
  `a_body_restore_rewrites_the_text_and_objects_it_names` (adapter round trip:
  apply, then the stored inverse restores the exact JSON),
  `a_body_restore_with_a_stale_expect_is_refused_before_mutation` (stale or
  missing `expect`, missing `objects`, objects that do not match the text → all
  `invalid`, nothing written).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 49, 54, 27 and 20 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.12 A paragraph move is an addressable structural operation (run 2026-11-09)

`TASK.md` §8 and §5.10 listed `para.moveUp`/`para.moveDown` among the commands
whose undo is a barrier, and a command also made every replica replay the move
itself. A move is a reordering of blocks, so it is now one structural
`move_block` operation with two explicit session ids: **head `227dc176`** on
`feature/realtime-collaboration`, with this entry's docs commit on top (no push,
CI unchanged).

**The operation.** `kind:"structural"`,
`payload: {"op":"move_block","count":n,"neighbour":<nodeId>}`; the operation's
`target.nodeId` is the first block of the run. The run and the neighbour swap
places in their container: the neighbour is either directly above the run (the
run moves up past it) or directly below it (the run moves down past it). Both
addresses are ids, so every replica names the same content and recomputes the
same plan from the same document; the authoritative delta repeats the operation
with the run's id:

```json
{"op":"move_block","nodeId":42,"count":1,"neighbour":17}
```

`count` is at most `MAX_MOVE_BLOCKS` (64). Every id at or below the container
follows the block it names (`NodeIds::permute_children`), so the caret holder of
a moved paragraph — or of a paragraph inside a moved table — keeps its id and
keeps typing in its own block.

**Refused before anything changes.** An id the session never issued is
`invalid`; an id the room retired is `gone`; a neighbour in another container,
not directly beside the run, or naming the run itself is `invalid`; a run that
leaves the container, a block that is not a paragraph (a table in the way) or a
paragraph carrying a section break is `invalid`; `count` outside `1..=64` is
`invalid`. Each refusal leaves the document byte-identical and the room version
where it was, which the tests assert. On the command path, `para.moveUp` at the
top of a story is refused by the engine before it runs, so the room answers
`invalid` with no version and no undo step.

**How it travels.** Two paths, one delta:

- The client's `collab-doc::diff` recognizes the move from the `Arc` pointers:
  a move leaves every block shared with the document before it, so the pointers
  say exactly which block went where. A run of up to 64 blocks yields one
  `move_block`; a wider reorder is reported as unsupported (the client falls
  back to the command) instead of being mistaken for a chain of text
  replacements — `diff` refuses a pure reorder it cannot name.
- The engine command path (`para.moveUp`/`para.moveDown`, and `outline.moveUp`/
  `moveDown` which run them) runs the command with the engine's own rules, then
  reads the document difference back. When it is one captured adjacent move, the
  step's delta becomes the structural operation, not the command, and the id map
  is rebased; replicas apply the same structural delta through
  `wordcraft_collab_engine::apply_delta`.

**Undo and redo.** A move's inverse is the neighbour crossing the run one block
at a time, each step again an adjacent `move_block` with explicit ids; the room
wraps the step's deltas, so a run of `n > 1` blocks undoes as one `transaction`
delta and a one-block move as one `move_block`. The author applies that answer
through the path of §5.9 and the peer applies the same JSON, so both end at the
identical document. Redo replays the original operation and the order returns.

**A foreign edit is `undo-conflict`.** The step reports every block of the run
and the neighbour, with their subtrees, in `Effect::touched`. An edit another
user makes in any of them after the move makes undo answer `undo-conflict`,
leaves both documents byte-identical and does not move the room version; the
step stays on the stack and the session stays `InSync` (§5.9). Typing into a
block the move did not displace does not block the undo.

**What stays a barrier.** A selection wider than 64 blocks is not captured as a
move: the command still runs, but the step keeps the command delta and its undo
answers `unsupported`. Every other command that reorders or rewrites blocks
remains a barrier (§5.10); `para.sort` is lifted out of this list by §5.13 and
the outline level commands by §5.14.

**Tests.**

- `crates/collab/src/ids.rs`: `a_permutation_swaps_two_blocks_with_their_subtrees`
  (a paragraph and a table swap; the table's rows, cells and cell paragraph
  keep their ids, and the map answers by the new path).
- `crates/collab-doc/src/tests.rs`:
  `moving_a_paragraph_past_its_neighbour_is_one_structural_operation` (delta
  shape, order, ids follow the blocks),
  `undo_of_a_two_block_move_lands_as_one_transaction_and_redo_returns_the_order`
  (two `move_block` ops in one `transaction`, order restored, redo moves again),
  `a_move_with_an_unknown_or_retired_id_is_refused_before_mutation` (unknown,
  non-adjacent, past the end → `invalid`; a retired id → `gone`; document and
  version unchanged every time),
  `a_foreign_edit_of_a_moved_block_makes_undo_a_conflict` (the displaced block,
  not the target),
  `moving_a_paragraph_diffs_to_move_block`,
  `moving_two_paragraphs_diffs_to_one_move_with_the_same_addresses`,
  `a_reorder_wider_than_the_bound_is_refused_instead_of_becoming_text_edits`
  (66 blocks → `Unsupported`),
  `a_replica_applies_a_move_and_keeps_the_ids_of_the_blocks_it_moved`.
- `crates/collab-engine/src/tests.rs`:
  `a_paragraph_move_lands_as_one_structural_operation_and_undoes` (the command
  yields a `move_block` delta; a replica that never ran it converges from the
  delta, undoes and redoes),
  `a_move_command_at_the_top_of_a_story_is_refused_before_mutation` (document,
  version and undo stack unchanged),
  `a_move_on_an_unknown_node_is_refused_before_mutation`,
  `a_move_over_more_than_64_blocks_stays_an_undo_barrier` (66 paragraphs move,
  undo → `unsupported`).
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s): `a_paragraph_move_undo_returns_the_order_at_its_author_and_the_peer`
  (the plan is structural; undo and redo reach the author once and the peer as
  the same delta),
  `a_foreign_edit_of_a_moved_paragraph_refuses_the_undo_without_touching_the_document`
  (Bob types into the displaced paragraph; both documents byte-identical, room
  version unchanged).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 50, 62, 31 and 22 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.13 A paragraph sort is an addressable permute operation (run 2026-11-09)

§5.12 left `para.sort` (and `para.promote`/`demote`) among the commands whose
undo is a barrier and whose forward effect every replica had to replay as a
command. A sort is a reordering of a contiguous run of paragraphs, so it is now
one structural `permute` operation with an explicit list of session ids:
**head `828fa107`** on `feature/realtime-collaboration`, with this entry's docs
commit on top (no push, CI unchanged).

**The operation.** `kind:"structural"`,
`payload: {"op":"permute","nodeId":<first id in the new order>,"order":[<id>…]}`.
The order list names the blocks of one container in their new order; together
the ids must fill one consecutive run of at most 64 paragraphs. The list is the
whole address: every replica resolves the same ids to the same content and
performs the same permutation. A duplicate id, an unknown id (`invalid`), a
retired id (`gone`), ids from different containers or a list with a gap between
the indexes, a table in the run and a run that leaves the container are refused
before the document or the id map changes; the room version does not move. A
direct `permute` wider than 64 blocks is `invalid`; a `para.sort` over a wider
selection is not normalized at all (see the barrier below).

**One transaction, same JSON at the author and the peer.** The authoritative
delta wraps the operation in the step's unit:

```json
{"op":"transaction","ops":[{"op":"permute","nodeId":42,"order":[42,17,9]}]}
```

Every id below the container follows the block it names
(`NodeIds::permute_children`), so the caret holder of a sorted paragraph — or of
a paragraph inside a sorted table, once the run is addressable — keeps its id
and keeps typing in its own block. The client's `collab-doc::diff` recognizes a
pure reorder from the `Arc` pointers: when the blocks are the same and only
their order differs, it emits one `permute` with the ids in the new order. An
adjacent swap still travels as `move_block` (§5.12); a reorder wider than 64
blocks stays `Unsupported` and falls back to the command.

**Undo and redo.** A sort's inverse is the same `permute` with the ids in their
old order. Because the operation handler always emits its delta as a
`transaction`, both the forward step and its inverse reach every replica as one
delta — the author applies the answer to its own document and the peer applies
the same JSON. Redo replays the original operation and the sorted order returns.

**A foreign edit is `undo-conflict`.** The step reports every block of the run
with its subtree in `Effect::touched`. Any later edit another user makes in one
of them makes undo answer `undo-conflict`, leaves both documents byte-identical
and does not move the room version; the step stays on the stack and the session
stays `InSync` (§5.9).

**The command path and a sort that changes nothing.** `para.sort` runs through
the engine as before (its natural key, language and selection rules decide the
order). The adapter captures the selected run before the command and names the
resulting order by ids after it; a sort that reorders nothing still becomes an
identity `permute`, so the step is addressable instead of a barrier. A run
wider than 64 paragraphs, a run that crosses a container or a paragraph without
a session id is not captured: the command keeps its delta and its undo answers
`unsupported`. `para.promote`/`demote` change outline levels and are lifted out
of the barrier list by §5.14.

**A fresh node map after a permutation.** `SyncState::set_nodes` used to rebase
ids one by one; with a permutation that can erase a key another id just wrote
(one id's old key is another's new key). It now replaces all paths in one pass
(`NodeIds::rebase_many`), so a move or a sort followed by the server's node map
leaves the client's map consistent.

**Tests.**

- `crates/collab-doc/src/tests.rs`:
  `sorting_blocks_is_one_transactional_permute` (delta shape, new order, ids
  follow their blocks), `undo_of_a_sort_lands_as_one_transaction_and_redo_returns_the_order`,
  `a_permute_with_an_unknown_or_retired_id_is_refused_before_mutation` (unknown,
  a gap, a duplicate, another container → `invalid`; retired → `gone`; document
  and version unchanged every time), `a_permute_with_a_table_in_the_run_is_refused_before_mutation`,
  `a_foreign_edit_of_a_sorted_block_makes_undo_a_conflict` (the step stays on
  the stack), `sorting_blocks_diffs_to_one_permute_with_explicit_ids`,
  `a_sort_wider_than_the_bound_is_refused_instead_of_becoming_text_edits`
  (67 blocks → `Unsupported`),
  `a_replica_applies_a_permute_and_keeps_the_ids_of_the_blocks_it_moved`.
- `crates/collab-engine/src/tests.rs`:
  `a_paragraph_sort_lands_as_one_structural_operation_and_undoes` (the command
  yields a transaction `permute`; a replica that never ran it converges, undoes
  and redoes), `a_sort_that_changes_nothing_is_still_an_addressable_step`,
  `a_sort_over_more_than_64_blocks_stays_an_undo_barrier`,
  `a_permute_on_an_unknown_node_is_refused_before_mutation` (document and
  version unchanged, no undo step).
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s):
  `a_paragraph_sort_undo_lands_as_one_transaction_at_its_author_and_the_peer`
  (the plan is one structural `permute`; undo and redo reach the author once and
  the peer as the same transaction delta),
  `a_foreign_edit_of_a_sorted_paragraph_refuses_the_undo_without_touching_the_document`
  (Bob types into a sorted paragraph; both documents byte-identical, room
  version unchanged).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 50, 70, 35 and 24 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.14 An outline level is an addressable structural operation (run 2026-11-09)

§5.13 lifted `para.sort` out of the barrier list and left `para.promote`/`demote`
there: they change a paragraph's outline level, which no primitive could name and
no replica could replay backwards (a relative step would move twice). The level
is now one structural `set_para_level` operation with an explicit list of
session ids and levels: **head `cd4c623c`** on
`feature/realtime-collaboration`, with this entry's docs commit on top (no push,
CI unchanged).

**The operation.** `kind:"structural"`,
`payload: {"op":"set_para_level","items":[{"id":42,"level":2},{"id":17,"level":null}]}`.
`level: 1…9` gives the paragraph `Heading{n}`, `level: null` makes it body text.
A level change reorders nothing and moves no block, so the block paths and the id
map stay exactly as they are; the caret holder of a levelled paragraph keeps its
id and keeps typing in its own block.

An item may also name its exact `style` (a style id, or `null` for the default
style) and direct `outline` level (`0…8`). That is how the inverse restores a
paragraph the level alone does not describe: a `Quote` promoted to a heading, or
a paragraph whose `Heading2` style carried the direct outline level `0` (level 1
by the engine's rule, style `Heading2`). An `outline` without a named `style` is
refused, because the implied styles (`Heading{n}` and body text) always clear the
direct outline level.

**One transaction, the same JSON at the author and the peer.** The authoritative
delta wraps the operation in the step's unit:

```json
{"op":"transaction","ops":[{"op":"set_para_level","items":[{"id":42,"level":2}]}]}
```

The command path reads the difference back: it walks the document before and the
document after the command side by side (blocks the command did not touch share
their `Arc`) and collects every paragraph whose style or direct outline level
changed. A command that also changed text, runs, objects, the mark, the section,
another paragraph property, a list level, a table's shape or the block order is
not captured at all: the step keeps its command delta and stays a barrier. The
`outline.promote`/`demote` pair (which runs `para.promote`/`demote`) and the
Outlining tab's absolute levels (`outline.promoteToHeading1`,
`outline.demoteToBody`, `outline.level`) are captured the same way. A delta that
carries a level change and a `permute` in one transaction applies all or none:
the replica path already runs a transaction on a scratch copy (§5.13).

**Undo and redo.** The inverse is one `set_para_level` naming the levels before
the step, with the exact `style`/`outline` where the earlier level does not imply
them. The room wraps the step's operations, so both the forward step and its
inverse reach every replica as one `transaction` delta: the author applies the
answer to its own document and the peer applies the same JSON, ending at the
identical `Document`. Redo replays the original command and the levels return.

**A foreign edit is `undo-conflict`.** The step reports every paragraph whose
level it changed in `Effect::touched`. Any later edit another user makes in one
of them makes undo answer `undo-conflict`, leaves both documents byte-identical
and does not move the room version; the step stays on the stack and the session
stays `InSync` (§5.9).

**The command path, a step that changes nothing, and the bound.** `para.promote`
and `para.demote` run through the engine as before (Word's rules decide each
paragraph's target level, including the heading before a promoted body
paragraph); the adapter names the result. A `set_para_level` that names the
levels already in place is still an addressable step: the document does not
change but the delta, the undo step and the inverse exist, so nothing falls back
to a command barrier. A step wider than `MAX_LEVEL_PARAGRAPHS` (64 paragraphs)
is not captured:
the command runs, keeps its delta and its undo answers `unsupported`. A
selection with a list item changes a numbering level too and stays a barrier for
the same reason.

A replica applying the delta adds the built-in `Heading{n}` style (and its
linked character style) when its document lacks it — the command path does the
same before it writes the style id — so a file that brought no heading styles
formats the same on every replica.

**Tests.**

- `crates/collab-doc/src/tests.rs`:
  `levelling_paragraphs_is_one_transactional_level_step` (delta shape, styles
  after the step, ids unchanged), `undo_of_a_level_step_lands_as_one_transaction_and_redo_returns_the_levels`,
  `undo_restores_a_style_the_level_does_not_imply` (`Quote` → heading → `Quote`),
  `undo_restores_a_direct_outline_level_the_style_does_not_imply` (`Heading2`
  with outline level 0),
  `a_level_step_with_an_unknown_retired_or_unaddressable_id_is_refused_before_mutation`
  (unknown, duplicate, level 0/10, `outline` without `style`, a stale path
  outside the container → `invalid`; retired → `gone`; document and version
  unchanged every time), `a_level_step_on_a_table_is_refused_before_mutation`,
  `a_level_step_wider_than_the_bound_is_refused_before_mutation` (65 items),
  `a_foreign_edit_of_a_levelled_block_makes_undo_a_conflict` (the step stays on
  the stack), `a_level_step_that_names_the_current_levels_is_an_identity_step`,
  `a_replica_applies_a_level_step_and_keeps_the_styles`.
- `crates/collab-engine/src/tests.rs`:
  `a_promote_lands_as_one_level_operation_and_undoes` (the command yields a
  transaction `set_para_level`; a replica that never ran it converges from the
  delta, undoes and redoes),
  `a_promote_over_a_selection_levels_every_paragraph_in_one_transaction`,
  `a_promote_over_more_than_64_paragraphs_stays_an_undo_barrier`,
  `a_promote_with_nothing_to_promote_is_refused_before_mutation` (document,
  version and undo stack unchanged),
  `a_level_step_that_changes_nothing_is_still_an_addressable_step`,
  `a_level_step_and_a_permute_land_as_one_transaction_on_a_replica` (both
  operations land; a refused transaction changes nothing).
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s): `a_promote_undo_lands_as_one_transaction_at_its_author_and_the_peer`
  (the plan is the command; the delta is one transaction, undo and redo reach the
  author once and the peer as the same delta, documents identical),
  `a_foreign_edit_of_a_promoted_paragraph_refuses_the_undo_without_touching_the_document`
  (Bob types into the promoted paragraph; both documents byte-identical, room
  version unchanged).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 50, 80, 41 and 26 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

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
