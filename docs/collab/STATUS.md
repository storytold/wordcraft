# Real-time collaboration: implementation status

- Date: 2026-11-11
- Branch: `feature/realtime-collaboration` (fork `copoka/wordcraft`)
- PR: <https://github.com/storytold/wordcraft/pull/5834> (ready for review
  since 2026-11-10 14:57Z, no longer a draft)
- Base: upstream `main` `4b0d3995`; the branch was rebased onto it (§5.4)
- HEAD at this writing: `50c68e3a` (§5.34, the finding-10 fix), with this page
  on top of it. Unlike the earlier pushes, `50c68e3a` is **delivered**: the
  local `HEAD` of `upstream/`, the fork branch
  (`git ls-remote fork refs/heads/feature/realtime-collaboration`) and the
  `headRefOid` of PR #5834 all answer
  `50c68e3ad07cd4114004ea292b1d6eb4739a2231` (§5.35). The fifteen commits after
  `af9a43a2` — `2a6f39c7` and the code+documentation pairs of §5.28
  (`925bb0c9`, `3225e644`), §5.29 (`264afd18`, `c6e3df4a`), §5.30 (`bcca9bce`,
  `37af31cb`), §5.31 (`bf632378`, `4e95d929`), §5.32 (`c1582864`, `88aca38b`),
  §5.33 (`3b05f844`, `64f05201`) and §5.34 (`b963a8db`, `50c68e3a`) — are in
  the fork and in the PR. The branch is 94 commits over `4b0d3995` and 95 with
  this page; the diff is 69 files, +30473/−22. The branch was pushed on
  2026-11-10 at 14:38Z (`15c9625b`), around 17:14Z (`077b4056`), at 20:15Z
  (`af9a43a2`) and around 03:27Z on 2026-11-11 (`50c68e3a`); at each point the
  local `HEAD`, the fork branch and the `headRefOid` of PR #5834 were the same
  commit (§5.21, §5.24, §5.26, §5.27, §5.35).
- The PR is `MERGEABLE` and `BLOCKED`, and its `reviewDecision` is empty: the
  branch merges cleanly, and `BLOCKED` is the required review plus the
  unapproved workflows. It was marked ready for review at 14:57:53Z, and the
  title reads "Real-time collaboration: authoritative sync server, document
  adapter, client core". The fork git source is gone from the manifest and the
  lock (§5.24), and the three `printing.yml` steps are green on the pushed tree
  at the build host (§5.25).
- CI has still not executed on any pushed head. The five runs created at
  03:27:31Z on `50c68e3a` — `Printing`, `SonarCloud`, `Packaging lint`, `Nix`
  and `CI` — are all `action_required`, as are the five on `077b4056` (17:14Z)
  and the five on `af9a43a2` (20:15:37Z), until a maintainer approves them
  (§5.2, §5.26, §5.27, §5.35). A maintainer had approved the workflows for
  `467948fa`; there `Nix` and `SonarCloud` passed and `Printing` failed on a
  stale `Cargo.lock`, not on code (§5.23).
- CodeRabbit has not finished on `50c68e3a`: its check started at 03:27:51Z
  and was `in_progress` when this page was written, and no review exists for
  that head. The last completed review is `5848804525` on `af9a43a2`
  (20:25:20Z), whose two Minor findings are answered in §5.27; §5.35 records
  the state of the new one, and has no findings to triage yet.
- Protocol: `rfc.md`; server: `server.md`; run book: `README.md`; host bridge:
  `host-bridge.md`; operator view and the callback contract:
  `production.md`.

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
  an operation moves paths or adds ids (ADR-0002). A command that allocates ids
  (a comment, a footnote, a shape, a new table) carries the new, moved and
  retired entries in `Applied.nodes` instead, so replicas keep addressing the
  new nodes without a snapshot (§5.20).
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
  newcomer's role on its first frame. A successful join is announced to the
  peers already in the room with an empty-state presence frame sent right after
  `welcome`, so the role is there before the newcomer's first presence; the
  welcome list still gives every role to the client that joins.
- Snapshot = model JSON + source `.docx`; `Document::kept` is taken back from the
  source on restore (ADR-0011). Server snapshots every 500 operations, on commit
  and when the last peer leaves; a final snapshot is written on SIGTERM.

### Document adapter (`crates/collab-doc`)

- Primitives: `insert_text`, `delete_range`, `replace_range`, `split_para`,
  `merge_para`, `insert_block` (with the block body and its container path),
  `remove_block`, `move_block` (a run of blocks swapped with the neighbour named
  by its session id, §5.12), `permute` (a contiguous run of paragraphs put in
  the order of an explicit id list, §5.13), `set_para_level` (the outline level
  of a list of paragraphs named by session id, §5.14), `set_para_list` (a
  bulleted or numbered list at an explicit level, with the numbering definition
  it creates, §5.15), `set_para_multilevel` (one multilevel definition with each
  paragraph at its own level `0..=8`, §5.16), `set_para_restart_numbering` (one
  list restarted at an explicit start, §5.16), `remove_blank` (a list of empty
  paragraphs named by session id, with a restore form that puts them back with
  their own ids, §5.17), `insert_row`, `remove_row`, and
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
  levels (§5.14). A list membership change is the same: `para.bullets`/
  `para.numbering` travel as a command and the server names the exact list state
  and the numbering definition it allocated (§5.15); `para.multilevel` and
  `para.restartNumbering` do too, keeping their own op names and the multilevel
  levels or the restarted instance with its explicit start (§5.16).
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
- The Bullets and Numbering buttons (`para.bullets`/`para.numbering`) are
  normalized after the command to one `set_para_list` structural operation
  naming each changed paragraph's session id with its exact list state (the
  numbering instance, level, style and the indents the command cleared) plus
  the numbering definition the command created; replicas replay the same
  transaction delta, undo restores the earlier list references and drops the
  definition the step created (only while nothing uses it), redo lists again
  with the same numbering id, and a foreign edit in any listed paragraph makes
  it `undo-conflict` (§5.15). A step wider than 64 paragraphs, a paragraph
  inside a table, a paragraph without a session id, or a command that also
  changed anything but those fields keeps the command delta.
- The Multilevel List button (`para.multilevel`) is normalized after the
  command to one `set_para_multilevel` structural operation naming each changed
  paragraph's session id, level (`0..=8`), style and indents plus the
  nine-level numbering definition the command allocated; replicas replay the
  same transaction delta and undo restores the earlier list state and drops the
  definition the step created (§5.16). Restart numbering
  (`para.restartNumbering`) is normalized to one
  `set_para_restart_numbering` operation naming the paragraphs that moved to
  the fresh instance and the instance with its explicit start overrides, so
  undo puts the earlier numbering references back and drops the instance
  (§5.16). Both keep the command delta for a step wider than 64 paragraphs, a
  paragraph inside a table or one without a session id.
- Delete Blank Paragraphs (`para.removeBlank`) is normalized after the command
  to one `remove_blank` structural operation naming the empty paragraphs it
  took out; replicas replay the same transaction delta, undo puts the same
  paragraphs back with their runs, properties and session ids in one step, and
  redo removes them again (§5.17). The step reports the surviving paragraphs
  beside each removal in `touched`, so a foreign edit there makes undo
  `undo-conflict`. A command that removed nothing is an identity: no delta and
  no step on the stack, so it is not a barrier either. A step that also changed
  text or a kept block, touched a paragraph inside a table or one without a
  session id, or removed more than 64 paragraphs keeps the command delta.

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
- Participants window: status, role, participants, commit button (owner), the
  commit result/warnings and — when the room published the file — the cabinet's
  version and etag (§5.18); after a `409` it shows the conflict and the owner's
  "Save a copy…" action, which writes the room's `.docx` as a copy (§5.19).
  Remote carets **and selection shading** are drawn on the canvas in the
  participant's colour.
- The Commit button commits through the room when a session is live (the room
  publishes the file, §5.18); the host-owned `wordcraft:commit` path is the
  fallback for an app without a session to commit through.
- A plan or a submission that fails does not leave the session reporting
  `InSync`: it moves to `Status::Diverged`, keeps the reason for the panel,
  refuses the next edit and the commit with that reason, and asks for a resync
  (`Pause`, `plan_command`, `submit_plans`; ADR `0006`, §5.33). The same two
  helpers serve the web session, so both transports pause the same way.
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
- Commit goes through the room when the session is live: the server builds the
  `.docx` and posts it to the cabinet (M87, §5.18), and every peer's panel shows
  the cabinet's version/etag from `committed`. `wordcraft:commit` (no `bytes`:
  the contract's `bytes?` allows a host that assembles the file itself) is the
  fallback when the app has no live session, and `wordcraft:committed` from the
  host is still recorded.

### Server (`apps/collab-server`)

- `tokio`/`axum` WebSocket, JWT first message (HS256 dev / ES256 public key),
  `aud`/`app`/`role` checks, `exp` re-checked on **every** frame.
- One room per document with a lease; a different cabinet session may join as
  owner or after the old session has been silent for `COLLAB_LEASE_TTL_SECS`
  (default 120) and has no connected peer.
- Presence is fanned out to the other participants and never journaled; a viewer
  may send it. Presence is not echoed to its author. A successful join is
  announced to the other peers after `welcome` with the same frame (empty
  `state`, `role` from the JWT), so the role arrives before the newcomer's first
  presence.
- A `client_id` belongs to the connection that registered it: only a connection
  with the same `sub` and `sid` may take it over (a reconnect), the old socket is
  retired with `close`, and `leave` removes a peer only when the connection id
  matches — a stale socket cannot remove its replacement (§5.32, ADR WC-0005).
- Commit is owner-only, checks `expectedVersion`, writes the `.docx` in memory,
  returns SHA-256 and warnings, and stores the source bytes for the next snapshot.
- With `COLLAB_CABINET_URL`, the commit is also published to the cabinet
  (`cabinet.rs`, §5.18): `{fileId, bytes, sha256, version, expectedVersion?}` →
  `{version, etag}`; the etag is stored with the snapshot, every peer receives
  `committed{cabinet:{version, etag}}`, `409` resyncs the room and keeps the
  built file for the owner's `saveCopy` (§5.19), `401`/`413` are one refusal, a
  transient failure is retried with the same body.
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
| Changes converge without loss or duplicates | **done** for the silent-divergence class (§5.28, §5.29), with a named price: a forced resync can cost unconfirmed keystrokes | `a_replica_applies_the_deltas_and_converges`, socket tests, `three_replicas_converge_on_a_hundred_page_document_and_one_undoes_under_live_edits` (§5.22) pass. Finding 1 is closed in `925bb0c9`: a positional text primitive whose `baseVersion` is behind another client's change of the same paragraph is refused `stale` (retryable) and its author resyncs (`a_text_edit_on_a_paragraph_another_client_changed_since_its_base_is_refused`, `a_stale_positional_edit_resyncs_instead_of_diverging`; ADR `0001-positional-edit-guard.md`). Finding 5 is closed in `264afd18`: an operation dropped from a full peer outbox marks the peer stale, wakes its stuck write and ends the connection, and every write to a peer is bounded by the heartbeat period, so the client reconnects and resyncs instead of staying stale (`a_peer_that_stops_reading_is_closed_and_resyncs_instead_of_staying_stale`, `a_full_outbox_marks_the_peer_stale_once`; ADR `0002-stale-peer-close.md`). Finding 2 is closed in §5.30: a stored snapshot that cannot be read or parsed is told apart from a missing one and refuses the join instead of becoming an empty room that is written over it (`a_corrupt_snapshot_refuses_the_join_and_is_not_overwritten`, store tests; ADR `0003-corrupt-snapshot-refused.md`). The remaining §5.26 findings are separate failure modes: 3 (room seeding) is closed in §5.31, 6 (connection identity) in §5.32 and 8 (a failed edit stayed local while the session said `InSync`) in §5.33 — a session whose plan or submission failed now leaves `InSync` for `Status::Diverged`, refuses the next edit and the commit with the reason, and resyncs (`a_command_that_cannot_be_planned_pauses_the_session_before_a_commit`, `a_command_that_cannot_be_submitted_pauses_the_session_before_a_commit`; ADR `0006-diverged-session-pauses.md`); 10 (the token renewal that replaced the client) is closed in §5.34 — a repeated `session` for the live connection renews its credentials in place, so the unconfirmed edit and the participant list survive (`a_repeated_session_renews_the_same_connection_and_keeps_an_unconfirmed_edit`; ADR `0007-token-renewal-keeps-the-connection.md`); 9 (per-document connections) remains open, named in §4 |
| Reconnect and resync | done | `a_reconnecting_client_catches_up_from_the_journal`, `reconnect.rs`, gap→snapshot |
| Per-user undo/redo | done | `undo_and_redo_over_the_socket_change_the_committed_text`, `undo_round_trips_through_the_room`; table inverses §5.8; the author applies its own answer once and a refused undo shows §5.9; character and paragraph formatting commands carry an exact inverse §5.10; a paragraph move, a paragraph sort, an outline level step, a bulleted/numbered list step, a multilevel list step, a restarted list step and a blank-paragraph removal are addressable structural operations that undo and redo §5.12, §5.13, §5.14, §5.15, §5.16, §5.17 |
| Viewer cannot edit | done | `a_viewer_cannot_write` + client gate |
| Commit creates a new version, original kept | done | commit/409/`warnings[]`; §5.18 publishes the built file to the cabinet (M87), stores its etag with the snapshot and shows `cabinet.version`/`etag` to every peer; a `409` offers the room's file as a copy without overwriting the cabinet (§5.19); cabinet owns versions |
| Single-user mode intact; feature behind a flag | done | flag off by default; 1137 UI tests |
| RFC, protocol, tests, documentation | done | `rfc.md`, `server.md`, `README.md`, this page |
| PR to upstream | open and ready for review (not draft) since 2026-11-10 14:57Z; the head is delivered | PR #5834; `headRefOid` `50c68e3a` equals the fork branch and the local `HEAD` (§5.35); `MERGEABLE`/`BLOCKED` with an empty `reviewDecision`; the five runs on that head wait in `action_required` for a maintainer's approval (§5.2, §5.35) |
| Web host bridge | done; `craft-web-host` pinned to revision `7666e2b1`, fetched from `storytold/craft-libs` (§5.24) | `host.rs`, `collab-web`, `host-bridge.md` |
| Two clients through a cabinet-like host page | done headlessly: one browser, two iframes | §5.1: session → edit → presence → commit → `committed`; 30 ops, 0 rejects |
| Two real machines and the real cabinet storage | not run | §5.1 "What remains for a person" |

## 3. Verification

Run from `upstream/`:

```bash
cargo test -p wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine \
  -p wordcraft-collab-client -p wordcraft-collab-ws -p wordcraft-collab-app \
  -p wordcraft-collab-web -p wordcraft-web \
  -p wordcraft-collab-server          # 329 tests, 1 ignored (LibreOffice gate)
cargo test -p wordcraft-collab-server -- --ignored   # commit → LibreOffice
cargo test -p wordcraft-engine collab::              # 5 flag/remote tests
cargo test -p wordcraft-ui-egui --lib                # 1137 tests
cargo xtask fmt / clippy / test / doctest / assets / layers / wasm
CRAFT_I18N_DIR=~/craft-i18n CRAFT_FONTS_DIR=~/craft-fonts cargo xtask ci   # all 9 steps
```

Results at this writing:

- §5.28, the slice this page records: on the build host, `cargo fmt --all --
  --check` EXIT=0, `cargo clippy` over `collab`, `collab-client`, `collab-doc`,
  `collab-app` and `collab-server` (`--all-targets -D warnings`) EXIT=0, and
  `cargo test` over the same five crates — 284 passed, 0 failed, 1 ignored
  (`collab` 59, `collab-doc` 116, `collab-client` 19, `collab-app` 39,
  `collab-server` 51/1). No `wordcraft-ui-egui`, engine or workspace-wide run
  in this pass: no file outside the five crates and `docs/collab` changed.
- 350 protocol/adapter/client/server tests, all green; the LibreOffice commit
  gate stays the one ignored test. The nine crates in this run:
  `collab` 59, `collab-doc` 113, `collab-engine` 58, `collab-app` 38,
  `collab-client` 17, `collab-web` 5, `collab-ws` 1, `wordcraft-web` 9,
  `collab-server` 50 (their socket scenarios included). §5.18 added 19: 13 unit
  tests of the callback client (`cabinet.rs`), 1 store round-trip of the cabinet
  etag, 4 server scenarios with a fake cabinet on loopback and 1 wire test that
  an older `committed` still parses.
- §5.22 added 1 test to `collab-engine` (57 → 58). Slice run on the build host
  `agents` from the committed tree: `cargo fmt --all -- --check` EXIT=0,
  `cargo clippy -p wordcraft-collab-engine --all-targets -- -D warnings` EXIT=0,
  and `cargo test` over the nine crates above — 349 passed, 0 failed, 1 ignored
  (the LibreOffice commit gate); `collab-engine` 58/0/0 in 1.23 s, 1.23 s of it
  the new test.
- §5.3 join announcement added 1 `collab-app` test (37 → 38) and rewrote the
  server's three-client socket test. Slice run on the build host `agents` from
  this tree: `cargo fmt --all -- --check` EXIT=0, `cargo clippy -p
  wordcraft-collab -p wordcraft-collab-app -p wordcraft-collab-server
  --all-targets -- -D warnings` EXIT=0, `cargo test` over the same three crates
  EXIT=0 — 147 passed, 0 failed, 1 ignored (`collab` 59, `collab-app` 38,
  `collab-server` 50 passed / 1 ignored, the LibreOffice commit gate). The new
  socket test times out when the announcement call is removed (checked).
- Full gate on the final tree (`cargo xtask ci`, build host `agents`, with
  `CRAFT_I18N_DIR` and `CRAFT_FONTS_DIR` from the host): all 9 steps passed —
  fmt, clippy (`--workspace --all-targets -D warnings`), test (5991 passed, 0
  failed, 11 ignored), i18n-report, language, doctest, assets, layers and wasm.
- Full gate on the same tree (`cargo xtask ci`, build host `agents`, with
  `CRAFT_I18N_DIR` and `CRAFT_FONTS_DIR` from the host): all 9 steps passed —
  fmt, clippy (`--workspace --all-targets -D warnings`), test (5969 passed, 0
  failed, 11 ignored), i18n-report, language, doctest, assets (48 files),
  layers (27 crates) and wasm (22 crates).
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
  and the CI run did not modify it. A later commit added a server dependency
  without the lock edge and broke `--locked`; §5.23 records the fix
  (`39f2e1a4`) and the gates that re-verified it.
- After the `opId`-window change (§5.5) the affected crates were re-run on the
  build host from the committed tree: `cargo test -p wordcraft-collab` (45
  passed), `cargo test -p wordcraft-collab-server -p wordcraft-collab-client`
  (29 + 13 passed, 1 ignored: the LibreOffice commit gate),
  `cargo clippy -p wordcraft-collab -p wordcraft-collab-server
  -p wordcraft-collab-client --all-targets -- -D warnings` and
  `cargo fmt --all -- --check`, all green.

## 4. Known limitations

- **The craft-libs pin is a revision outside `storytold/main`.** `crates/web-host`
  in `storytold/craft-libs` has the six collaboration messages on
  `feat/collab-session-messages`, commit
  `7666e2b100d8cf12998f9b57d3b036964cc3189c`, not merged into `storytold/main`
  yet. Per ADR-0016 §3 `apps/wordcraft-web/Cargo.toml` pins `craft-web-host` to
  that revision **fetched from `storytold/craft-libs`** (the head of
  storytold/craft-libs#716; `git ls-remote` and the GitHub API both serve the
  commit, and cargo fetched it by SHA from storytold), and `Cargo.lock` records
  the storytold URL (§5.24). No git dependency comes from another owner: the
  `git-sources` CI job fails on one. The wasm app builds and tests on that pin
  (see §3 and `host-bridge.md` §"Build note"). After the craft-libs PR merges,
  pin the merge commit; nothing else in the app changes.
- Web collab has been run with two clients only inside one headless browser
  (two iframes through a cabinet-like host page, §5.1). Two real machines, the
  real cabinet page and its storage, and the visible commit status stay manual.
- **Convergence's silent-divergence class is closed** (§5.26 findings 1 and 5,
  §5.27, §5.28, §5.29). Finding 1: the room refuses a positional text primitive
  whose base is behind another client's change of the same paragraph (`stale`,
  retryable) and its author resyncs (`docs/collab/adr/0001-positional-edit-guard.md`,
  `925bb0c9`). Finding 5: a document frame dropped from a peer's full outbox
  marks that peer stale, cancels the write it is stuck in and ends its
  connection; every write to a peer is bounded by the heartbeat period, so the
  client reconnects with `lastVersion` and receives the journal tail or a
  snapshot (`docs/collab/adr/0002-stale-peer-close.md`, `264afd18`). The price is
  named in both ADRs: a forced resync can cost the unconfirmed keystrokes of the
  client that resyncs, which continues from the snapshot. Finding 2 no longer
  turns a damaged document into an empty one: the store tells "never written"
  from "stored and cannot be opened", and the second refuses the join while
  leaving `model.json`/`meta` and `prev/` untouched
  (`docs/collab/adr/0003-corrupt-snapshot-refused.md`, §5.30). Finding 3 no
  longer turns an open document into an empty page: a room is built from the
  copy its first writer brings, and a join that brings none is refused
  (`docs/collab/adr/0004-room-without-snapshot-seeded.md`, §5.31). Finding 6 no
  longer lets a participant claim another's `client_id`, and a stale connection
  can no longer remove the connection that replaced it: a peer is removed by the
  connection that registered it, and only the same account in the same cabinet
  session may take an id over — a reconnect
  (`docs/collab/adr/0005-client-id-belongs-to-a-connection.md`, §5.32). Finding
  8 no longer leaves a failed edit in a session that reports `InSync`: a plan or
  a submission that fails moves the session to `Status::Diverged`, which pauses
  editing and refuses a commit with the reason, and a resync replaces the local
  document with the room's confirmed one
  (`docs/collab/adr/0006-diverged-session-pauses.md`, §5.33). Two findings of
  §5.26 stay open, each with its own slice: the connection is app-wide while
  documents are per window (9), and a session event replaces the client and
  drops its unconfirmed edits (10).
- A step of several operations is now one transaction (every inverse or none);
  character and paragraph formatting commands carry an exact inverse (§5.10),
  including the two that rewrite the characters, `format.changeCase` and
  `format.enclose`, whose inverse restores the paragraph's body (§5.11), and a
  paragraph move, a paragraph sort, an outline level step, a list step, a
  multilevel list step, a restarted list step and a blank-paragraph removal are
  addressable structural operations with an exact inverse (§5.12, §5.13, §5.14,
  §5.15, §5.16, §5.17); every other command still has none and acts as an undo
  barrier. The
  `opId` dedup window travels with the snapshot (§5.5), so a retry across a
  restart is
  deduplicated; the window is bounded at 10 000 answers, so a retry after more
  than that (or of an operation accepted after the last snapshot, which the
  restore cannot replay either) is not covered.
- A command that allocates ids no longer costs a snapshot: the accepted answer
  carries `nodes` (`changed`, `removed`, `overflow`), so replicas adopt the ids
  of a comment, a footnote, a text box or a new part and keep editing (§5.20).
  The snapshot remains the fallback when the command changed an existing
  story's size (every later path shifted), when the delta passed
  `MAX_NODE_DELTA = 4096` entries (`overflow: true`), when a replica receives
  such a command without a delta (an older server) and on any version gap. An
  id-allocating command must land at the head; a stale one is refused `stale`
  and the author resyncs instead of keeping ids minted on an older state.
  Because the delta is not journaled, a retry answered from the `opId` window
  carries no node list; the join that precedes a retry sends the whole map.
- Table cells are addressable now, and so are the table's columns and merged
  cells (§5.7): a column insert/remove, a cell merge and a cell split travel as
  structural operations that reindex cell ids instead of retiring them, so the
  caret of a client holding a cell id survives. What still travels as a command
  is a text box, formatting and anything the diff cannot express; a formatting
  command's inverse is a `set_para_format` primitive (§5.10, §5.11), a paragraph
  move is a `move_block` structural operation (§5.12), a sort is a `permute`
  (§5.13), an outline level change is a `set_para_level` (§5.14), a list
  membership change is a `set_para_list` (§5.15), a multilevel list is a
  `set_para_multilevel`, a restarted list is a
  `set_para_restart_numbering` (§5.16) and a multi-paragraph blank removal is a
  `remove_blank` (§5.17), the rest
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
- A replica that skips its own structural delta — the author of a command, whose
  change is already in its document — keeps the ids that step retired in its
  map: `SyncState::set_nodes` leaves disappeared ids alone on purpose (the delta
  path tombstones them) and the author never applies its own delta. The document
  and the version are unaffected; the server refuses an operation on a stale id
  `gone` and the client resyncs, and a snapshot rebuilds the map from scratch.
  The author's map stays correct for live nodes (§5.17 fixed one way it did
  not).
- The cabinet callback (`production.md`) is `http://` only: the room sits behind
  the ingress, which terminates TLS. A URL that is not `http://` or that has no
  `{sessionId}` stops the server at startup with a message that says so.
- A callback whose `200` is lost is retried with the same `expectedVersion`, so a
  cabinet that did store the first attempt answers `409`; the room reports the
  conflict rather than creating a second version (§5.18). Callback idempotency
  is the cabinet's next step (mail-frontend M87). The first commit of a room (or
  after its snapshot was removed) sends no `expectedVersion`, so an external
  edit in that window is overwritten.
- The token is read from the environment on desktop; the cabinet must push a fresh
  token through the host bridge when it arrives. On the web the push is handled:
  a repeated `session` for the live connection renews the credentials of the
  running client instead of replacing it (ADR WC-0007, §5.34). The desktop has
  no host bridge yet, so it reads the environment once at start.
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
   listed there. The room-side cabinet callback is done (§5.18) and a `409`
   offers the room's file as a copy (§5.19); what remains for it is operational:
   the cabinet's ticket must carry the numeric `fileId` in `doc`, and the shared
   service token must be in both `.env`s.
2. Get a maintainer's approval for the runs on the PR head. The head is
   delivered: the local `HEAD`, the fork branch and the PR `headRefOid` are all
   `50c68e3a`, the PR is `MERGEABLE`, and it was marked ready for review at
   14:57:53Z, so the draft flag is gone (§5.21, §5.26, §5.35). A maintainer had
   approved the workflows for the earlier head `467948fa`: `Nix` and
   `SonarCloud` passed, `Printing` failed on a stale `Cargo.lock` rather than on
   code, and `39f2e1a4` fixes it (§5.23). Every push since started a fresh set
   of five runs that wait in `action_required`, so nothing has executed on any
   of them; the three `printing.yml` steps have been run green on the pushed
   tree at the build host instead (§5.25). The latest set is the one on
   `50c68e3a`, created at 03:27:31Z — `Printing`, `SonarCloud`, `Packaging
   lint`, `Nix` and `CI`, all `action_required` — and CodeRabbit's check on
   that head was still `in_progress` at 03:27:51Z (§5.35). This page is a local
   commit on top of `50c68e3a` and is not pushed; the envoy role cannot push,
   so it is one fast-forward for the owner.
3. Optional hardening: property primitives or an offline queue. (The join
   broadcast that carries the role before the first presence frame is done:
   §5.3. The multi-operation undo transaction is done:
   `a_step_of_several_operations_lands_as_one_transaction`; the `opId` window
   survives a snapshot restore: §5.5; table-cell ids are done: §5.6; columns,
   merges and splits are primitives: §5.7; their inverses are done and an undo
   answer lands at its author: §5.8, §5.9; command inverses for the frequent
   character and paragraph formatting commands are done: §5.10; a paragraph
   move is an addressable structural operation that undoes: §5.12.)
4. Manual acceptance (desktop): two desktops over the real server, a 100+ page
   document, commit and reopen in Word/LibreOffice.

### 5.3 Role in presence and on join (run 2026-11-09; join announcement 2026-11-10)

§5.1 left one visible gap: a peer that was already connected learned about a
newcomer only from `presence`, which carried the caret and not the role, so its
participant list showed `participant` until the next welcome. `TASK.md` §3 asks
for "a participant list with roles". The role now travels with presence, and a
join announcement tells the room's earlier peers who joined before the newcomer
sends anything.

- **Wire.** `ServerMsg::Presence` gains `role` (`#[serde(default)]`, so a frame
  from an older server still parses). The join announcement reuses the same
  frame with an empty `state`, so a peer built before the field parses it too.
  The client's `Presence` message is unchanged: a client cannot name a role, the
  server stamps it from the JWT claim (ADR-0008), the same authority that gates
  write and commit.
- **Server.** `process_frame` copies the authenticated role into every presence
  broadcast. On a successful join, after `welcome`, `announce_join` sends the
  other peers an empty-state presence frame with the newcomer's `client_id` and
  `role`. The newcomer is not echoed its own join: it learns the existing peers
  from `welcome`, which already lists their roles. The room keeps the role per
  peer (`Peer.role`) for the welcome list and the lease checks; nothing new is
  trusted from the wire.
- **Client.** `Event::Presence` carries the role. `collab-app` folds a frame
  into the participant list through `apply_presence` (shared by the web session
  in `collab-web`): a known peer gets the role updated in place, a new peer is
  added with it, and an empty role (older server) keeps the role already known
  instead of overwriting it with `participant`. The announcement needs no client
  change.
- **UI.** Nothing else was needed: the desktop Collaboration panel already
  renders `id · role` from `ParticipantView`, and the host bridge already sends
  `hostParticipants` with `role` to the cabinet page.

- **Docs.** `server.md` and `rfc.md` now spell the control-message fields as the
  wire does (`client_id`, `session_id`, `expected_version`, …): serde's
  `rename_all` on an enum renames variants, not their fields, so the envelope
  structs stay camelCase while the message fields are snake_case. The presence
  row carries `client_id` and the server-added `role`.

**Test.** `a_join_announcement_gives_earlier_peers_the_newcomer_role`
(`apps/collab-server/src/tests.rs`) connects three real WebSocket clients —
`owner`, `editor`, `viewer` — checks that each welcome lists the roles already
present, then has the third join and asserts that both peers that joined
earlier receive an empty-state presence frame with the newcomer's role before
the newcomer sends any presence. The presence frames that follow still carry the
sender's raw role. The test times out when the announcement is removed
(verified). `presence_carries_the_sender_role` (client),
`presence_gives_a_new_peer_its_role` (collab-app: an empty-state frame adds the
peer with its role) and the `apply_presence` tests (placeholder replacement,
role-less frame, role-less join announcement) cover the halves without a socket;
`an_older_presence_without_a_role_still_parses` (wire) pins both the first
presence frame and the join frame of a server built before the field.

### 5.4 Rebase onto upstream `main` `4b0d3995` (run 2026-11-09)

GitHub reported PR #5834 as conflicting: `main` had moved from the branch's
merge base `2a0a20ae` to `4b0d3995` (279 files, a new page command API among
them). The branch was rebased in place, no commit dropped:

| | |
|---|---|
| before | the pre-rebase head — 32 commits over `2a0a20ae` |
| after | `d39ef64c` — the same 32 commits over `4b0d3995` |
| backup ref | `backup/pre-rebase-<old head>` (local) |
| push | made later, after the gates, with a manual `--force-with-lease` (§5.21) |

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

**Push.** After the gates above were green, the rebased branch was force-pushed
to `copoka/wordcraft` (`git push --force-with-lease fork
feature/realtime-collaboration`); PR #5834 then reported `MERGEABLE` on the same
base (§5.21).

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
adjacent paragraph move, §5.13 lifts the paragraph sort, §5.14 lifts the
outline level commands (`para.promote`/`demote`), §5.15 lifts the Bullets and
Numbering buttons (`para.bullets`/`para.numbering`) and §5.16 lifts the
Multilevel List and Restart numbering buttons. Commands that move text past
more than 64 blocks
(`para.moveUp`/`moveDown` over a wider selection), strip text
(`para.removeBlank`, `para.convertBreaks`, the space/indent text tools),
allocate numbering definitions another way or otherwise
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
reorder blocks in other ways, allocate numbering definitions another way or
change more than
a paragraph's
body and formatting are unchanged from §5.10 (an adjacent paragraph move is
lifted in §5.12; the Multilevel List and Restart numbering buttons are lifted in
§5.16). The client's diff fallback is
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
remains a barrier (§5.10); `para.sort` is lifted out of this list by §5.13, the
outline level commands by §5.14 and the Bullets and Numbering buttons by §5.15.

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
of the barrier list by §5.14; `para.bullets`/`para.numbering` change list
membership and are lifted out by §5.15.

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
the same reason (the Bullets and Numbering buttons themselves, which set the
list membership, are §5.15).

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

### 5.15 A list membership is an addressable structural operation (run 2026-11-09)

§5.14 lifted the outline level commands out of the barrier list and left
`para.bullets`/`para.numbering` there: they allocate a numbering definition,
which no primitive could name and no replica could replay backwards (a toggle
would flip again, and the definition the command allocated would differ across
replicas). List membership is now one structural `set_para_list` operation with
an explicit list of session ids, a kind and a level: **head `4480bc13`** on
`feature/realtime-collaboration`, with this entry's docs commit on top (no push,
CI unchanged).

**The operation.** The forward form names the paragraphs, the kind
(`bullets` or `numbering`) and the list level (`0..=8`):

```json
{"op":"set_para_list","ids":[42,17],"kind":"bullets","level":1}
```

The adapter reuses an existing definition whose nine levels are the canonical
levels of that kind, or creates one with the document's own deterministic
allocator (`free_id` over the numbering table, the same function the engine's
`para.bullets` uses). The rule is a pure function of the document, so the same
payload on two replicas that hold the same document picks the same abstract and
`num` ids; a redo after undo reuses the definition the first run created instead
of allocating a second one. A paragraph that already carries a list of the
requested kind at the requested level is left untouched (style and indents
included), so a repeated step is an identity: the delta still exists, the
document does not change and no definition is allocated.

**The exact form.** The command path and the inverse write each paragraph's
complete list-facing state — the numbering instance (or `null` to inherit, `0`
to leave the list), the level, the style and the four indents the list commands
clear — plus the definitions the step adds:

```json
{"op":"set_para_list","items":[{"id":42,"num":3,"level":1,"style":"ListParagraph",
  "indentLeft":null,"indentFirst":null,"indentLeftChars":null,"indentFirstChars":null}],
 "define":[{"abstract":{…},"num":{…}}]}
```

A replica that never ran the command creates the definition with the identical
ids and levels from `define`, so both replicas hold the same `Document` JSON,
numbering table included. A definition that collides with an existing one (same
id, different body) is refused; a `num` a step names must be defined by the
document or by the same step.

**One transaction, the same document at the author and the peer.** The
authoritative delta wraps the exact operation in the step's unit:

```json
{"op":"transaction","ops":[{"op":"set_para_list","items":[…],"define":[…]}]}
```

The author applied the command optimistically; the peer applies the delta. Both
end at the identical document because the command is deterministic on identical
input and the delta names what it allocated. Undo is one exact `set_para_list`
naming the state before the step plus `undefine` of the `num` the step created:
the definition is dropped only when no paragraph, style or other list still
points at it, so an edit another user made in the meantime keeps its label. The
earlier list references come back, and because the definition is dropped, redo
recreates it with the same id and ends at the same JSON as the first run.

**A foreign edit is `undo-conflict`.** The step reports every paragraph whose
list state it changed in `Effect::touched`. Any later edit another user makes in
one of them makes undo answer `undo-conflict`, leaves both documents
byte-identical and does not move the room version; the step stays on the stack
and the session stays `InSync` (§5.9).

**The command path and what stays a barrier.** `para.bullets` and
`para.numbering` run through the engine as before; the adapter walks the
document before and after (blocks the command did not touch share their `Arc`)
and captures the step only when the difference is the list-facing state of at
most `MAX_LIST_PARAGRAPHS` (64) paragraphs. A command that also changed text,
runs, objects, the mark, the section, another paragraph property, the block
order or an existing numbering definition is not captured at all: the step
keeps its command delta and stays a barrier. The same holds for a changed
paragraph inside a table (the operation addresses top-level paragraphs only) or
one without a session id, and for a step wider than 64 paragraphs. The direct
operation refuses those cases before the document changes: an unknown id
(`invalid`), a retired id (`gone`), a table, a row, a cell or a paragraph inside
a table, a duplicate id, an empty set, a level outside `0..=8`, an unknown kind,
a colliding definition and a wider step are refused with the document and the
room version unchanged.

**Tests.**

- `crates/collab-doc/src/tests.rs`:
  `listing_paragraphs_is_one_transactional_list_step` (delta shape, one
  definition, level 1, `ListParagraph`, ids unchanged),
  `two_replicas_that_apply_the_same_list_payload_allocate_the_same_numbering_id`
  (identical documents and the same `num` on both),
  `a_repeated_list_step_is_an_identity_step` (document unchanged, no second
  definition), `undo_of_a_list_step_lands_as_one_transaction_and_redo_returns_the_list`
  (undo restores the exact earlier JSON including the numbering table, redo
  returns the listed JSON), `a_list_step_with_an_unknown_retired_or_unaddressable_id_is_refused_before_mutation`
  (unknown, duplicate, empty, level 9, unknown kind, stale path → `invalid`;
  retired → `gone`; document and version unchanged every time),
  `a_list_step_on_a_table_or_inside_one_is_refused_before_mutation`,
  `a_list_step_wider_than_the_bound_is_refused_before_mutation` (65 ids),
  `a_foreign_edit_of_a_listed_block_makes_undo_a_conflict` (the step stays on
  the stack), `a_replica_applies_a_list_step_and_creates_the_definition`.
- `crates/collab-engine/src/tests.rs`:
  `a_bullets_command_lands_as_one_list_operation_and_undoes` (the command yields
  a transaction `set_para_list` with its definition; a replica that never ran
  it converges from the delta, undoes and redoes),
  `a_numbering_command_over_a_selection_lists_every_paragraph_in_one_transaction`,
  `a_list_command_over_more_than_64_paragraphs_stays_an_undo_barrier`,
  `a_list_step_on_a_table_is_refused_before_mutation` (the table and a cell
  paragraph → `invalid`, document and version unchanged),
  `a_list_step_that_names_the_current_list_is_an_identity_step`.
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s): `a_bullets_undo_lands_as_one_transaction_at_its_author_and_the_peer`
  (the plan is the command; the delta is one transaction with the same numbering
  id at both replicas; undo and redo reach the author once and the peer as the
  same delta, documents identical), `a_foreign_edit_of_a_listed_paragraph_refuses_the_undo_without_touching_the_document`
  (Bob types into the listed paragraph; both documents byte-identical, room
  version unchanged).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 50, 89, 46 and 28 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.16 Multilevel lists and restart numbering are addressable structural operations (run 2026-11-09)

§5.15 lifted a list membership out of the barrier list and left the Multilevel
List button and Restart numbering there: a multilevel definition numbers nine
levels together (and the `legal` and `outline` kinds allocate different
definitions), and a restart allocates a second numbering instance, which no
primitive could name or replay backwards. Both are now addressable operations
of the same list template — payload → normalized plan → adapter → confirmed
delta → two replicas: `set_para_multilevel` and `set_para_restart_numbering`.
**Head `129ecc9e`** on `feature/realtime-collaboration`, with this entry's docs
commit on top (no push, CI unchanged).

**The multilevel operation.** The forward form names the paragraphs with their
own level and the multilevel kind:

```json
{"op":"set_para_multilevel","kind":"legal",
 "levels":[{"id":42,"level":0},{"id":17,"level":2}]}
```

`kind` is `legal` (Word's `1.`, `1.1.`, `1.1.1.`) or `outline` (`I.`, `A.`,
`1.`, …); every level is `0..=8`. The adapter reuses an existing definition
whose nine levels are the canonical levels of that kind, or creates one with
the document's own deterministic allocator (`free_id` over the numbering
table), so the same payload on two replicas that hold the same document picks
the same abstract and `num` ids. A paragraph that already carries that
definition at the level it names is left untouched (style and indents
included), so a repeated step is an identity: the delta still exists, the
document does not change and no definition is allocated.

**The restart operation.** The forward form names the paragraphs that move to
the fresh instance and the first number it shows:

```json
{"op":"set_para_restart_numbering","ids":[42,17],"start":3}
```

Every named paragraph must already be in the same list (a restart restarts one
list) and `start` is `1..=100_000`, the engine's own bound on "Set Numbering
Value". The adapter allocates the new `num` with the document's allocator and
copies the source's abstract list and level overrides, then sets its start
overrides to `(level, start)` for every level the named paragraphs use, so the
allocation is a pure function of the document and both replicas pick the same
id. A restarted instance the named paragraphs already sit in whose start
overrides are exactly the requested ones makes the step an identity: the
document does not change and no second instance is allocated.

**The exact form.** Both operations keep the `set_para_list` exact form — the
same `items`, `define` and `undefine` — only under their own op name:

```json
{"op":"set_para_multilevel","items":[{"id":42,"num":3,"level":1,"style":"ListParagraph",
  "indentLeft":null,"indentFirst":null,"indentLeftChars":null,"indentFirstChars":null}],
 "define":[{"abstract":{…},"num":{…}}]}
{"op":"set_para_restart_numbering","items":[{"id":42,"num":4,"level":0,"style":"ListParagraph",…}],
 "define":[{"abstract":{…},"num":{"id":4,"abstractId":1,"startOverrides":[[0,3]],…}}]}
```

A replica that never ran the command creates the definition with the identical
ids and levels from `define`, so both replicas hold the same `Document` JSON,
numbering table included. A definition that collides with an existing one (same
id, different body), a `num` a step names without a definition, a level outside
`0..=8`, a zero, negative or too large start, an empty set, a duplicate id and a
step wider than `MAX_LIST_PARAGRAPHS` (64) are refused before the document or
the version changes.

**One transaction, the same document at the author and the peer.** The
authoritative delta wraps the exact operation in the step's unit:

```json
{"op":"transaction","ops":[{"op":"set_para_multilevel","items":[…],"define":[…]}]}
```

Undo is one exact op of the same name naming the state before the step plus
`undefine` of the `num` the step created: the definition is dropped only when
no paragraph, style or other list still points at it. The earlier references
come back, and because the definition is dropped, redo recreates it with the
same id and ends at the same JSON as the first run.

**A foreign edit is `undo-conflict`.** The step reports every paragraph whose
list state it changed in `Effect::touched`. Any later edit another user makes in
one of them makes undo answer `undo-conflict`, leaves both documents
byte-identical and does not move the room version; the step stays on the stack
and the session stays `InSync` (§5.9).

**The command path and what stays a barrier.** `para.multilevel` and
`para.restartNumbering` run through the engine as before; the adapter walks the
document before and after (blocks the command did not touch share their `Arc`)
and captures the step when the difference is the list-facing state of at most 64
paragraphs, naming the delta with the command's own op
(`set_para_multilevel`/`set_para_restart_numbering`) instead of
`set_para_list`. A command that also changed text, runs, objects, the mark, the
section, another paragraph property, the block order or an existing numbering
definition is not captured: the step keeps its command delta and stays a
barrier. The same holds for a changed paragraph inside a table, one without a
session id, and a step wider than 64 paragraphs (its undo answers
`unsupported`). The direct operations refuse those cases before the document
changes: an unknown id (`invalid`), a retired id (`gone`), a table, a row, a
cell or a paragraph inside a table, a duplicate id, an empty set, a level
outside `0..=8`, an unknown multilevel kind, a start of 0 or above 100 000, a
paragraph that is not in a list, a restart that names two different lists, a
colliding definition and a wider step.

**Tests.**

- `crates/collab-doc/src/tests.rs`:
  `a_multilevel_step_lists_each_paragraph_at_its_level` (delta shape, one
  nine-level definition, levels 0/1/2, `ListParagraph`),
  `two_replicas_that_apply_the_same_multilevel_payload_allocate_the_same_numbering_id`,
  `a_repeated_multilevel_step_is_an_identity_step` (document unchanged, no
  second definition, no `define`),
  `undo_of_a_multilevel_step_lands_as_one_transaction_and_redo_returns_the_levels`,
  `a_multilevel_step_with_a_bad_level_or_id_is_refused_before_mutation` (empty,
  level 9, duplicate, unknown kind, a missing level, unknown id → `invalid`;
  retired → `gone`), `a_multilevel_step_on_a_table_is_refused_before_mutation`,
  `a_multilevel_step_wider_than_the_bound_is_refused_before_mutation` (65 ids),
  `a_foreign_edit_of_a_multilevel_block_makes_undo_a_conflict`;
  `a_restart_step_moves_the_paragraphs_to_a_new_instance_with_an_explicit_start`
  (same abstract, new `num`, `startOverrides` [[0,4]]),
  `two_replicas_that_apply_the_same_restart_payload_allocate_the_same_numbering_id`,
  `a_repeated_restart_step_is_an_identity_step`,
  `undo_of_a_restart_step_lands_as_one_transaction_and_redo_returns_the_start`,
  `a_restart_step_with_a_bad_start_or_address_is_refused_before_mutation`
  (start 0, -1, 100 001, empty, duplicate, unknown, no start, not in a list,
  two lists → `invalid`; retired → `gone`),
  `a_restart_step_on_a_table_is_refused_before_mutation`,
  `a_restart_step_wider_than_the_bound_is_refused_before_mutation`,
  `a_foreign_edit_of_a_restarted_block_makes_undo_a_conflict`.
- `crates/collab-engine/src/tests.rs`:
  `a_multilevel_command_lands_as_one_multilevel_operation_and_undoes`,
  `a_restart_numbering_command_lands_as_one_restart_operation_and_undoes` (the
  command yields a transaction of its own op; a replica that never ran it
  converges from the delta, undoes and redoes),
  `a_multilevel_step_that_names_the_current_levels_is_an_identity_step`,
  `a_restart_step_that_names_the_current_start_is_an_identity_step`,
  `a_multilevel_command_over_more_than_64_paragraphs_stays_an_undo_barrier`,
  `a_restart_step_with_a_zero_start_is_refused_before_mutation`.
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s): `a_multilevel_undo_lands_as_one_transaction_at_its_author_and_the_peer`
  (the plan is the command; the delta is one `set_para_multilevel` transaction
  with the same numbering id at both replicas; undo and redo reach the author
  once and the peer as the same delta),
  `a_restart_numbering_undo_lands_as_one_transaction_at_its_author_and_the_peer`
  (the listing lands first, the restart moves both paragraphs to the same fresh
  instance, undo puts the source instance back and redo recreates the same one),
  `a_foreign_edit_of_a_multilevel_paragraph_refuses_the_undo_without_touching_the_document`
  (Bob types into the listed paragraph; both documents byte-identical, room
  version unchanged).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 50, 105, 52 and 31 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.17 Deleting blank paragraphs is an addressable structural operation (run 2026-11-09)

§5.16 left `para.removeBlank` in the barrier list: it deletes several blocks at
once, so no primitive named it and its undo could only say `unsupported`. It is
now one structural `remove_blank` operation with an explicit list of session
ids — payload → normalized plan → adapter → confirmed delta → two replicas,
the same template as §5.15/§5.16. **Head:** the code commit of this slice on
`feature/realtime-collaboration` is `d70b3a28` with a comment-only follow-up
`675b3f1a` (no push, CI unchanged); the docs commit adds this entry.

**The operation.** The forward form names the paragraphs by session id:

```json
{"op":"remove_blank","ids":[42,17]}
```

Every id must name a top-level paragraph of one story that is empty
(`Paragraph::is_empty`) and carries no section break — and the block after it
must be a paragraph too, because a story's last mark cannot be removed and a
paragraph before a table cannot be joined across it (the engine's own rule).
The operation removes exactly the named paragraphs, so it never touches a
non-empty one: a payload that names one is refused rather than filtered, which
keeps the address explicit.

**The exact form.** The inverse is the same operation with `restore`; each item
carries the paragraph's body, its id and the index it goes back to, highest
index first so every index stays valid as the paragraphs land:

```json
{"op":"remove_blank","story":"body",
 "restore":[{"id":17,"index":1,"block":{…}},{"id":42,"index":3,"block":{…}}]}
```

`index` in a restore is the index in the document *after* the step (every
earlier removal shifts it down by one), and `block` is the whole paragraph —
text, runs, mark, `ParaProps`, objects — so undo returns exactly the earlier
sequence of paragraphs. The restored paragraph takes its own session id back:
`NodeIds::rebase` revives a tombstoned id, so a client that held the id
addresses the same paragraph again (the same mechanism the table restores use,
§5.7/§5.8).

**One transaction, the same document at the author and the peer.** The
authoritative delta wraps the operation in the step's unit:

```json
{"op":"transaction","ops":[{"op":"remove_blank","story":"body",
  "items":[{"id":42,"index":3},{"id":17,"index":1}]}]}
```

The author applied the command optimistically; the peer applies the delta. Both
end at the identical document, and the node map moves identically (removed ids
tombstoned, later blocks shifted down, both restored ids rebased on undo). Undo
lands as one `transaction`, redo replays the forward op and removes the same
paragraphs again.

**A foreign edit is `undo-conflict`.** The step reports in `Effect::touched` the
surviving paragraphs beside every removal — the inverse puts the blank back
between them. Any later edit another user makes in one of them makes undo
answer `undo-conflict`, leaves both documents byte-identical and does not move
the room version; the step stays on the stack and the session stays `InSync`
(§5.9).

**The command path and what stays a barrier.** `para.removeBlank` runs through
the engine as before; the adapter walks the document before and after (blocks
the command did not touch share their `Arc`), reads the removed block indexes
back and names them by session id, then lets `plan_remove_blank` check every
one again. A command that also changed text, runs, objects, a kept block, the
block order, a table or another story, a paragraph inside a table, a paragraph
without a session id, and a step wider than `MAX_BLANK_PARAGRAPHS` (64) is not
captured: the step keeps its command delta and stays a barrier (its undo
answers `unsupported`). A command that removed nothing is an identity: no
delta, no step on the undo stack, so it is not a barrier either. The direct
operation refuses the same cases before the document changes: an unknown id
(`invalid`), a retired id (`gone`), a non-empty paragraph, a section break, a
last block, a table, a row, a cell, a paragraph inside a table, a duplicate id,
an empty set, two stories in one step and a wider step — the document and the
room version do not move.

**A map bug this slice found and fixed.** `NodeIds::shift_children` moved each
child's two map entries one at a time, so when consecutive children shifted into
each other's paths (removing several paragraphs, or any insertion that moves a
run) a later child could erase the forward entry an earlier one had just
written: the id resolved, but `get(key)` returned `None` and a client could no
longer address a live paragraph. Every old key is now removed before any new
one is written (the way `permute_children` already worked), with a regression
test over a run of children in both directions.

**Tests.**

- `crates/collab/src/ids.rs`:
  `shifting_a_run_of_children_leaves_every_forward_key_in_place` (every child of
  a shifted run is still addressable at its new path, both directions).
- `crates/collab-doc/src/tests.rs`:
  `removing_blank_paragraphs_is_one_transactional_blank_step` (delta shape, the
  highest index first, survivors keep their ids, removed ids retired),
  `a_replica_applies_a_blank_step_and_keeps_the_survivors_ids` (delta
  convergence and the replica's map), `undo_of_a_blank_step_lands_as_one_transaction_and_restores_the_paragraphs_and_their_ids`
  (the restore items carry body/id/index, the document JSON is byte-identical
  and both ids are revived at their own paths),
  `a_blank_step_with_a_bad_address_or_a_non_empty_paragraph_is_refused_before_mutation`
  (empty set, duplicate, unknown id, non-empty paragraph, id 0, no `ids`, the
  last block → `invalid`; document and version unchanged every time),
  `a_blank_step_on_a_paragraph_inside_a_table_is_refused_before_mutation`
  (a cell paragraph and the table itself), `a_blank_step_naming_a_retired_id_is_gone`,
  `a_blank_step_wider_than_the_bound_is_refused_before_mutation` (65 ids),
  `a_foreign_edit_of_a_neighbouring_paragraph_makes_undo_a_conflict` (the step
  stays on the stack).
- `crates/collab-engine/src/tests.rs`:
  `a_remove_blank_command_lands_as_one_blank_step_and_undoes` (the command
  yields a transaction `remove_blank`; a replica that never ran it converges,
  undoes with the same ids and redoes),
  `a_remove_blank_command_with_no_blank_paragraph_changes_nothing_and_is_no_barrier`
  (identity: no delta, no undo step),
  `a_remove_blank_command_over_more_than_64_paragraphs_stays_an_undo_barrier`
  (command delta, undo `unsupported`), `a_blank_step_on_a_table_is_refused_before_mutation`
  (a blank whose next block is a table and a cell paragraph → `invalid`).
- `crates/collab-app/src/tests.rs` (two replicas, `Room<SessionModel>` plus two
  `SyncState`s): `a_blank_undo_lands_as_one_transaction_at_its_author_and_the_peer`
  (the plan is the command; the delta is one `remove_blank` transaction; undo
  returns the exact earlier document and both replicas address the restored
  paragraphs by their old ids; redo removes them again),
  `a_foreign_edit_of_a_neighbouring_paragraph_refuses_the_blank_undo_without_touching_the_document`
  (Bob types into a neighbour; both documents byte-identical, room version
  unchanged).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test -p
wordcraft-collab -p wordcraft-collab-doc -p wordcraft-collab-engine -p
wordcraft-collab-app` — 51, 113, 56 and 33 passed; `cargo clippy` on those four
crates with `--all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean. No push was made and CI is unchanged.

### 5.18 Commit callback to the cabinet (run 2026-11-09)

The last piece between the room and `CRAFT_COLLAB_ENABLED=true`: the cabinet's
M87 contract (`mail-frontend docs/CUTOVER.md` §M87) says the room builds the
final document and posts it, because the cabinet owns versions, audit and
antivirus. Until this slice the server built the `.docx` and returned only its
digest; the web path posted `wordcraft:commit` to the host page. Now the room
itself is a client of `POST /api/v1/internal/office/sessions/{sessionId}/commit`.

**Who calls.** The sync server, not the app, holds the authoritative document
and the bytes; the desktop and the web build get the same answer over the
same socket. `fileId` and `sessionId` come from the JWT claims the cabinet
minted (`doc` and `sid`), so no new host message and no new field on the wire
were needed. `version` is the room's document version; `expectedVersion` is the
cabinet etag of the previous accepted commit, stored with the snapshot
(`meta`: `cabinet_version`, `cabinet_etag`) so a restart does not lose the
conflict check.

**Configuration.** `COLLAB_CABINET_URL` (with `{sessionId}`, `http://` only —
the ingress terminates TLS), `COLLAB_CABINET_TOKEN` (secret, redacted in
`Debug`, never logged), `COLLAB_CABINET_TIMEOUT_MS` and
`COLLAB_CABINET_ATTEMPTS`. Without a URL nothing changes: the commit stays
server-side and the host owns the storage. The operator view, the contract and
the rollout are in `production.md`.

**What each answer does.**

- `200 {version, etag}`: the etag is recorded and the snapshot rewritten; every
  peer (not only the author, as before) receives
  `committed {version, sha256, warnings, cabinet:{version, etag}}`; the panel
  shows the cabinet's version and etag next to the digest.
- `409`: the file changed outside the session. The room does not overwrite it
  and does not advance the etag; a retryable `error {code:"cabinetConflict"}`
  goes to every peer, so both replicas resync and the panel says the commit was
  refused.
- `401`/`403`: one `rejected {code:"forbidden"}` to the author, no retry — a
  wrong `COLLAB_CABINET_TOKEN` cannot be fixed by sending it again.
- `413`: one `rejected {code:"tooLarge"}`, no retry.
- `408`/`425`/`429`/`5xx`/network: the same body is sent again (2 more attempts
  by default, 200 ms then 800 ms); a success is reported like any other. When
  every attempt fails, the author gets `rejected {code:"invalid"}` with the
  reason and the room keeps the old expectation; the next Commit retries.
- Any other `4xx`: one `rejected {code:"invalid"}`.

**Client side.** `ServerMsg::Committed` grew an optional `cabinet` field (the
serde default keeps an older server's frame parseable), `Event::Committed`
carries it, and `collab-app`/`collab-web` expose the result as a `Commit`
struct instead of a tuple, so the panel draws the cabinet version/etag. The
Commit button now commits through the live session first; the host-owned
`wordcraft:commit` path remains the fallback for an app with no session, and the
host's `wordcraft:committed` is still recorded (`host-bridge.md`).

**Tests.**

- `apps/collab-server/src/cabinet.rs` (13 unit, no real cabinet):
  `the_contract_codes_are_classified` (200/2xx, 401/403, 409, 413, 408/425/429,
  5xx, 400/422/418),
  `only_plain_http_urls_with_a_host_parse` (https refused, credentials and IPv6
  handled), `the_body_carries_the_contract_fields` (base64 bytes, digest,
  room version, optional `expectedVersion`),
  `the_config_never_prints_the_token`,
  `an_accepted_commit_names_the_cabinet_version_and_etag` (path, method,
  bearer header, body),
  `a_conflict_is_not_retried`, `a_bad_token_is_not_retried`,
  `a_too_large_body_is_not_retried` (one request each),
  `a_lost_answer_is_retried_with_the_same_body` (equal bodies, so the retry
  cannot become a second version), `an_unreachable_cabinet_gives_up_after_the_attempts`,
  `a_cabinet_that_refuses_the_body_is_reported_without_a_retry`,
  `a_chunked_answer_is_decoded`,
  `an_accepted_answer_without_the_contract_fields_is_refused`.
- `apps/collab-server/src/store.rs`: `the_cabinet_etag_survives_a_restart`.
- `apps/collab-server/src/tests.rs` (two real WebSocket clients and a fake
  cabinet on loopback):
  `a_commit_is_published_to_the_cabinet_and_every_client_sees_its_version`
  (both peers get the cabinet copy; the second commit sends the first etag as
  `expectedVersion`; `fileId`/`sessionId` come from the claims),
  `a_cabinet_conflict_resyncs_every_client` (retryable error to both, one
  request, no etag advance),
  `a_refused_cabinet_token_is_reported_once_without_a_retry` (forbidden to the
  author, token not in the URL),
  `a_lost_cabinet_answer_is_retried_with_the_same_body` (both peers get the
  stored copy; two equal bodies).
- `crates/collab-client/src/tests.rs`:
  `a_commit_answer_carries_the_cabinet_copy_when_the_room_published_it` (the
  new frame parses, an older one without `cabinet` still parses as `None`).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test` on the
nine crates above — `collab` 51, `collab-doc` 113, `collab-engine` 56,
`collab-app` 33, `collab-client` 14, `collab-web` 5, `collab-ws` 1,
`wordcraft-web` 9, `collab-server` 47 (+1 ignored LibreOffice); `cargo clippy`
on `collab`, `collab-client`, `collab-app`, `collab-web`, `collab-server`,
`ui-egui` and `wordcraft-web` with `--all-targets -- -D warnings` clean;
`cargo fmt --all -- --check` clean; `cargo xtask wasm` ok (22 crates; the two
pre-existing `ui-egui` wasm warnings only); the full `cargo xtask ci` on this
tree passed all 9 steps (5969 workspace tests, 0 failed, 11 ignored).

**What is left for this path.** The cabinet's ticket must carry the numeric
`fileId` in `doc` (the dev room key `doc-1` is not a file id), the same service
token must be in the cabinet's `CRAFT_CALLBACK_TOKEN` and the room's
`COLLAB_CABINET_TOKEN`, and the value of a lost `200` is a `409` until the
cabinet implements callback idempotency. No push was made and CI is unchanged.

### 5.19 Save a copy when the cabinet refuses the commit (run 2026-11-09)

TASK.md §6 says a commit must not lose work when the file changed outside the
session: the user is offered "save a copy" or a merge. §5.18 delivered the
`409` → `cabinetConflict` resync, which left the owner with nothing to keep;
this slice adds the copy. The cabinet is still never overwritten.

**Server.** The `.docx` built for a refused commit is kept in `RoomState`
(`Refused {version, sha256, warnings, bytes}`; the bytes are an `Arc` so a
request does not copy the file). A new owner-only request
`ClientMsg::SaveCopy {op_id}` answers `ServerMsg::SaveCopy {version, sha256,
warnings, bytes, last}`, chunked at `SNAPSHOT_CHUNK` (512 KiB) and sent to the
asking socket only — never broadcast. No HTTP request is made: the cabinet sees
exactly the one `409` attempt. Editors get `forbidden`; with no refused commit
the answer is `rejected{conflict}`. A later accepted commit drops the copy;
a restart drops it too (it is not part of the document or the snapshot), and
the next Commit shows the conflict again. The metric
`wordcraft_collab_save_copies_total` counts the requests.

**Client.** `wordcraft_collab_client` maps the `cabinetConflict` error to a new
`Event::Conflict` (every replica still resyncs, as before), and the chunked
frame to `Event::SaveCopy`. The shared `CopyBuffer` in `collab-app` joins the
chunks and restarts when the version changes, so a late chunk of an older
answer cannot slip into a newer copy. `Collab` (desktop transport and the web
socket) keeps `conflict`, exposes `request_copy()`/`take_copy()`, and clears the
conflict when a commit is accepted. The participants panel shows the conflict
and, for the owner, a "Save a copy…" button; `WordApp::save_room_copy` writes
the room's bytes (not the local replica) under `<name> (copy).docx`: the native
build asks for a path through the same asynchronous file dialog and writes
with `std::fs::write`; the web build hands the bytes to `Services::download`
(the host page's save hook when there is one). The document is not marked
saved and the cabinet is untouched.

**Tests.** `apps/collab-server/src/tests.rs` (two real WebSocket clients and a
fake cabinet): `a_refused_commit_can_be_saved_as_a_copy_without_overwriting_the_cabinet`
(both peers get the conflict, the rooms' bytes arrive chunked, their SHA-256
matches, `PK\x03\x04`, the cabinet gets no second request, an editor is
refused) and `a_copy_without_a_refused_commit_is_refused`.
`crates/collab-client/src/tests.rs`: `a_cabinet_conflict_is_a_separate_event_and_resyncs`,
`any_other_error_still_resyncs`, `a_save_copy_chunk_parses_and_the_request_names_an_operation`.
`crates/collab-app/src/tests.rs`:
`refused_copy_chunks_join_until_last_and_restart_on_a_new_version`.
`crates/ui-egui/src/file_dialogs/tests.rs`:
`a_refused_commit_is_saved_as_a_copy_without_marking_the_document_saved`
(the suggested name, the picked path and the exact bytes, document stays
unsaved) and `cancelling_a_room_copy_writes_nothing`.

**Gates** (build host `agents`, `scripts/remote-build.sh`): the crate test
suites above are green, including the two-client server tests; `cargo clippy
--workspace --all-targets -- -D warnings` clean; `cargo fmt --all -- --check`
clean; the full `cargo xtask ci` passed all 9 steps (fmt, clippy, test,
i18n-report, language, doctest, assets, layers, wasm). On the build
host the full gate needs `CRAFT_FONTS_DIR` (the embedded CJK faces),
`CRAFT_I18N_DIR` (the `craft-i18n` checkout, which holds `language-data/`) and
`CRAFT_PROOF_EMBED=de,pl,pt,it,ru` (German/Portuguese hyphenation and grammar);
without them unrelated font/hyphenation tests fail. No push was made and CI is
unchanged.

### 5.20 The ids a command allocates travel with its answer (run 2026-11-09)

§4 said a command that allocates ids (comments, tables, shapes, footnotes, text
boxes) makes every replica ask for a snapshot: the id map of the new nodes was
issued only on the server. This slice puts that change on the wire. The
snapshot stays the fallback for a delta that cannot be applied and for a map
that cannot be described by new ids alone.

**Wire.** `Applied` gained `nodes: Option<NodeDelta>` (absent on older
servers, defaulted on parse). `NodeDelta { changed: [NodeRef], removed: [u32],
overflow: bool }` names the ids the operation issued or moved, the ids it
retired (a late operation on one is `gone`) and, when the change set passed
`MAX_NODE_DELTA = 4096`, that the replica must take a snapshot instead. The
list of commands that may allocate ids moved from `collab-app` into
`crates/collab/src/wire.rs` (`may_allocate_ids`), so the server and every
client classify a command the same way.

**Server (`Room`).** Before applying such a command, the room starts recording
in `NodeIds` (`start_delta`); `id_for`, `rebase`, `shift_children`,
`permute_children` and `remove` record what they touch, and `take_delta`
returns the change set (or `overflow`). After the command, when the model says
it changed nodes, the room indexes the new nodes (`Model::index_nodes`) and
attaches their ids to the **live** answer. A full map is still broadcast on
join and after a structural primitive; `handle_op` skips it when the answer
already carries `nodes`. The `opId` window and the journal store no node lists
(a reconnect gets the whole map in `Nodes`), which keeps the window bounded. A
command that resized an existing story cannot be described by new ids — every
later path shifted — so it keeps the snapshot path; `Model::story_shape`
(new method, default empty) is what tells the two apart. An id-allocating
command now requires `baseVersion == head`: otherwise the comment/part ids the
author minted optimistically could differ from the server's, so the operation
is refused `stale` and the client resyncs. `NodeIds::rebase_many` now advances
its id counter past every adopted id, so a locally issued id cannot collide.

**Client (`SyncState::apply`).** The delta is applied after the document
delta, rebasing `changed` and tombstoning `removed`. The author of a command
takes only `nodes` (its document already has the change). A command that may
allocate ids but arrives without `nodes`, or with `overflow`, returns
`Outcome::Resync` — the snapshot path — and the app asks for one.

**Tests.** `crates/collab/src/ids.rs`: `a_delta_names_the_ids_an_operation_issued_moved_and_retired`,
`an_id_retired_and_restored_by_one_operation_is_not_reported_removed`,
`a_delta_that_passes_the_bound_reports_overflow_instead_of_a_partial_map`,
`rebasing_an_id_above_the_counter_makes_the_next_id_free`.
`crates/collab/src/wire.rs`: `an_applied_carries_a_node_delta_and_an_old_answer_still_parses`.
`crates/collab/src/tests.rs` (fake model):
`a_command_that_appends_a_story_answers_with_the_new_ids` (the answer carries
the delta, the tail does not, the joined map does),
`an_id_allocating_command_needs_the_head_version`,
`a_command_that_grows_an_existing_story_keeps_the_snapshot_path`.
`crates/collab-engine/src/tests.rs`:
`a_comment_command_carries_the_ids_of_the_new_story_and_a_replica_can_address_them`
(the replica replays the command, adopts the same part id and the server
accepts an edit on it).
`crates/collab-app/src/tests.rs`:
`a_comment_id_allocation_keeps_both_replicas_editing_without_a_snapshot` (both
`Outcome::Quiet`, both one comment on the same part, Bob edits the comment
paragraph by the server's id and both converge),
`a_command_that_may_allocate_ids_without_a_delta_resyncs`,
`a_node_delta_that_overflowed_resyncs_instead_of_applying_a_partial_map`.
`apps/collab-server/src/tests.rs`:
`a_comment_allocation_reaches_a_peer_as_a_node_delta_and_it_keeps_editing` (two
real WebSocket clients: the broadcast carries the delta, the peer edits the new
comment paragraph and both get version 3).

**Gates** (build host `agents`, `scripts/remote-build.sh`): `cargo test` for
`wordcraft-collab`, `-doc`, `-engine`, `-client`, `-ws`, `-app` and `-server`
(all targets) green — 59 + 113 + 57 + 17 + 1 + 37 + 50 tests, 1 ignored on the
server; `cargo clippy` for the same crates `--all-targets -- -D warnings`
clean; `cargo fmt --all -- --check` clean. No push was made, the PR and CI were
not touched.

### 5.21 The pushed PR head matches the branch (run 2026-11-10)

The header, §5.2 and §5.4 still described the state before the force-push: a
fork copy on the pre-rebase head, a PR that could not merge until a push. The
push had already happened, so this entry records the reconciliation. Only
`docs/collab/STATUS.md` changed; no code, branch, ref or PR was touched.

**What the PR reports** (`gh pr view 5834 --repo storytold/wordcraft --json
headRefOid,mergeable,mergeStateStatus,headRepository,statusCheckRollup`):

| | |
|---|---|
| head | `467948fa105e17ff55194e52a2f31e8d1918c15f` |
| head repository | `copoka/wordcraft` |
| base | `4b0d3995` |
| mergeable | `MERGEABLE` (merge state `UNSTABLE`) |
| state | `OPEN`; draft at the time (the PR was marked ready for review at 14:57:53Z — §5.26) |
| check runs | `CodeRabbit` — completed, skipped |

`UNSTABLE` is the fork rule, not a failure: the four patch workflows (`CI`,
`Printing`, `Nix`, `SonarCloud`) all ended `action_required`, which is how
GitHub parks a fork's workflows until a maintainer approves the run. Nothing
has failed; nothing has run yet.

**Local check.** `git fetch fork feature/realtime-collaboration` leaves
`fork/feature/realtime-collaboration` at `467948fa`, the same commit as the
worktree head, and `git rev-list --count 4b0d3995..HEAD` is 69 — the number the
header states.

**Docs.** The header now names `467948fa` and the mergeable PR; §5.2 asks only
for the ready flag once CI has run; the §5.4 table and its closing paragraph
record the push instead of asking for it.

**Follow-up (2026-11-10).** The commit that carried this entry, `a785b611`, is
local: `git fetch fork feature/realtime-collaboration` still leaves the fork and
PR #5834 at `467948fa`. The header had said "`467948fa` plus this status
commit", which read as if the PR already carried it; it now names `a785b611` as
the last commit below this one and counts the local commits over the PR head.
Nothing was pushed; the branch, the PR and CI were left alone.

### 5.22 Three replicas on a hundred-page document, one of them undoing under live edits (run 2026-11-10)

TASK.md §8 asks that a large document stay usable with several participants.
`a_hundred_page_document_keeps_up_with_edits` covered 3000 paragraphs in one
room with one edit at a time — no replicas, no deltas, no undo. This entry adds
`crates/collab-engine/src/tests.rs::three_replicas_converge_on_a_hundred_page_document_and_one_undoes_under_live_edits`.
No production code changed.

**The document.** 3000 body paragraphs of ~170 characters, two lines each, so
the layout runs to 189 pages on the build host (`Session::layout().pages.len()`),
the same count on all three replicas. One room over one `SessionModel`.

**The clients.** `alice`, `bob` and `carol`: each a `Session` built from the same
file plus its own `NodeIds` (the ids are deterministic, ADR-0002), and each
applies the authoritative delta of every answer — its own and its peers'.

**Parallel edits.** 30 rounds; in every round the three clients edit one
paragraph of their own band (0–29, 30–59, 60–89) with a primitive `insert_text`
carrying a unique marker. Each sends the last version the server answered *it*,
so 89 of the 90 edits landed with `conflict: true` (ADR-0004) — only alice's
first, sent at head 0, did not; bob's and carol's first edits already carried a
base their peer had moved. Typing travels as a primitive because that is what
the client's diff produces (ADR-0002); a command is an undo barrier (§4) and
would make the undo half of this test impossible.

**The undo.** Alice undoes her newest step with a `history: undo` request on the
stale base her replica is at. The room accepts it — her step is newest and nobody
else touched that paragraph — the answer names the history step and carries the
inverse's delta, and all three replicas apply it.

**What is asserted.**

- 91 accepted operations with versions 1..=91 in order: one version per
  operation, no loss, no duplicate; head 91.
- All three replicas hold the server document (JSON equality) and version 91.
- Content: one marker in every edited paragraph and never twice; alice's last
  paragraph is back to its original text; the 2910 paragraphs outside the three
  bands are byte-equal to the original.
- The journal holds the whole session: a fresh replica that saw version 0
  replays all 91 deltas and lands on the same document.
- A retry of alice's first operation is answered from the `opId` window with
  version 1 and moves neither the head nor the document.
- Pagination: 189 pages on each of the three replicas, the same count.

**Run.** Build host `agents`, from the committed tree:
`cargo test -p wordcraft-collab-engine` 58 passed, 0 failed, 0 ignored (the new
test 1.23 s); the nine collaboration crates 349 passed, 0 failed, 1 ignored;
`cargo fmt --all -- --check` EXIT=0; `cargo clippy -p wordcraft-collab-engine
--all-targets -- -D warnings` EXIT=0; the full `cargo xtask ci` passed all 9
steps (5991 workspace tests, 0 failed, 11 ignored).

**Scope.** Headless only: no socket, no browser, no server process. It proves the
room and the engine adapter converge and paginate alike at this size; it does not
replace the two-client host run (§5.1) or the two-machine run still open in §2.

### 5.23 CI `Printing` failed on a stale `Cargo.lock`, not on code (run 2026-11-10)

A maintainer approved the paused workflows on PR head `467948fa`. Run
`42720338000` (`Printing`) failed in both matrix legs — `Print (macos-latest)`
and `Print (windows-latest)`, job `Native app and UI Clippy` — at

```
cargo clippy -p wordcraft -p wordcraft-ui-egui --all-targets --locked -- -D warnings
error: cannot update the lock file .../Cargo.lock because --locked was passed to prevent this
```

`Formatting` passed in the same job and the test steps were skipped; `Nix`
(`42720338487`) and `SonarCloud` (`42720338454`) on the same head completed
successfully, and the `CI` gate (`42720338676`) was still running.

**What cargo wanted to change.** Running `cargo metadata --format-version 1`
on the build host leaves a one-line diff: the `wordcraft-collab-server` entry in
`Cargo.lock` does not list `base64 0.23.1`, although the crate declares it
(`base64 = { workspace = true }`, `apps/collab-server/Cargo.toml:23`) and uses
it (`cabinet.rs` base64-encodes the commit body). `79ecdc34` added both the
dependency and `cabinet.rs` but not `Cargo.lock`; base64 0.23.1 was already in
the graph through other packages, so local builds resolve fine and cargo adds
the edge to the lock silently. Those builds run on the `agents` host
(`scripts/remote-build.sh`), whose `Cargo.lock` never comes back into the
repository, so CI's `--locked` was the first run to see the stale edge.

**The fix.** One line in `Cargo.lock` (commit `39f2e1a4`); no manifest changed.

**The three patch warnings are neither the failure nor the branch's.** The same
log prints `patch egui_demo_lib / egui_inspection / egui_system_font_provider
was not used in the crate graph`. The identical three warnings appear on
`origin/main` `4b0d3995` with `cargo metadata --locked` EXIT=0, and
`git diff origin/main -- Cargo.toml` is empty: they come from upstream's patch
table and are pre-existing noise. Deleting them would edit a manifest the PR
does not otherwise touch, so they were left alone.

**The fork URL is not the failure either.** The log shows `Updating git
repository https://github.com/copoka/craft-libs` completing before the lock
error. `craft-web-host` is pinned there deliberately (ADR-0016 §3, `7666e2b1`),
because the collaboration host messages are not in `storytold/craft-libs` yet;
dropping the pin would take the web session out of the build. That pin was a
separate, real problem — one owner for every git source — and it is fixed in
§5.24: the same commit is fetched from `storytold/craft-libs` and a CI job now
refuses a source from anywhere else.

**Gates** (build host `agents`, cargo 1.99.1, from the committed tree at
`39f2e1a4`): `cargo metadata --locked` EXIT=0 and does not modify the lock
(`git status --short Cargo.lock` clean after the commit); `cargo fmt --all --
--check` EXIT=0; the failing CI command itself, `cargo clippy -p wordcraft -p
wordcraft-ui-egui --all-targets --locked -- -D warnings`, EXIT=0 (50 s);
`cargo test --locked --all-targets` over the eight collaboration crates
(`collab`, `-doc`, `-engine`, `-client`, `-ws`, `-web`, `-app`, `-server`) 341
passed, 0 failed, 1 ignored (the LibreOffice commit gate); `cargo clippy
--locked` over the same crates `--all-targets -- -D warnings` EXIT=0.

Nothing else was pushed at that point: `39f2e1a4` reached CI with the next push
of the branch, which happened later at 14:38Z (§5.24).

### 5.24 CI: one owner for every git source — the fork pin is out of the lock, and a job keeps it out (run 2026-11-10)

§5.23 left two things of one class, both about delivery rather than the protocol:

1. **A git dependency came from a fork.** `apps/wordcraft-web/Cargo.toml:27` and
   `Cargo.lock:1236` named `git+https://github.com/copoka/craft-libs?rev=7666e2b1…`
   for `craft-web-host`. Every build, including CI's `--locked` resolution,
   cloned a fork branch; if that branch is force-pushed or deleted, `cargo
   metadata --locked` fails for everyone at once.
2. **Nothing checked where git dependencies come from.** `ci.yml` called only
   the shared `gate.yml`, which does not inspect sources, so a fork could come
   back unnoticed. pdfcraft and cadcraft had the same hole and closed it with a
   `git-sources` job (pdfcraft's `ci.yml`, job "Git dependencies come from
   storytold"); wordcraft did not.

**The same commit, fetched from `storytold`.** No code changed. The revision is
the head of storytold/craft-libs#716 — `7666e2b100d8cf12998f9b57d3b036964cc3189c`
— and `storytold/craft-libs` serves it: the GitHub API answers `200` with that
sha, and cargo fetched it by revision from storytold (the relock log prints
`Updating git repository https://github.com/storytold/craft-libs` and `Adding
craft-web-host v0.1.0 (https://github.com/storytold/craft-libs?rev=7666e2b1…)`).
pdfcraft pins the identical commit the same way (`apps/pdfcraft-web/Cargo.toml`),
so the five apps now describe one source.

**The lock was regenerated, not hand-edited.** On the build host `agents`,
`cargo metadata --format-version 1` rewrote exactly one line — the `source` of
`craft-web-host` — and nothing else moved: the package set and every other
`storytold/craft-libs`/`egui`/`winit`/`winprint.rs` revision are the same
(`git diff Cargo.lock` is `1 file changed, 1 insertion(+), 1 deletion(-)`). The
lock was then pulled back from the host, so the committed file is the one cargo
produced.

**The gate.** A new job `git-sources` ("Git dependencies come from storytold")
sits before `gate` in `ci.yml` and does three things: refuses source
replacement, `patch` and `paths` in `.cargo/config`/`.cargo/config.toml` (those
never show up in `Cargo.lock`); runs `cargo metadata --locked`, which catches a
lock out of date with the manifests; and parses `Cargo.lock` as TOML, failing on
any `git+` source that is not `git+https://github.com/storytold/`. It is the
same script pdfcraft runs, minus its Windows font-notice step, which wordcraft
does not have. It adds one `ubuntu-latest` job and changes no shared workflow.

**Proof, both directions** (build host `agents`, from the committed tree, the
job's own script extracted from `ci.yml` and run as-is):

| Run | Result |
|---|---|
| the job's script on this tree | EXIT=0 — `cargo metadata --locked` EXIT=0, and no git source outside `github.com/storytold/` |
| `grep -n 'github.com/copoka' Cargo.lock` | no match |
| manifest temporarily back on `copoka`, lock relocked from it | `cargo metadata --locked` **fails** (the lock/manifest drift is caught first), and the source scan exits 1 naming `git+https://github.com/copoka/craft-libs?rev=7666e2b1…` |
| both files restored from the saved copies | byte-identical to the committed tree; the script is EXIT=0 again |

**Gates on the same tree** (build host `agents`, `cargo 1.99.1`):

```
cargo fmt --all -- --check                                    EXIT=0
cargo clippy -p wordcraft-web -p wordcraft-collab-web \
    --all-targets -- -D warnings                              EXIT=0
cargo check -p wordcraft-web --target wasm32-unknown-unknown   EXIT=0
cargo xtask wasm                                              EXIT=0
cargo test over the nine collaboration crates                 350 passed, 0 failed, 1 ignored
```

The one ignored test is still the LibreOffice commit gate (needs `soffice`).
`dirty=3` in the gate log is this change in the working tree: the manifest, the
lock and `ci.yml` (this page came after).

**House rule 11.** `ci.yml` is CI, and the rule asks for CI changes outside a
feature or fix PR. The change is a repository guard rather than a shared
workflow, and it rides here because the pin and the gate close one hole
together. It is a self-contained commit (`ci: fail on a git dependency outside
storytold`); if a maintainer wants it as its own PR, it splits without touching
the rest.

The push followed at 14:38Z and carried both changes: local HEAD, the fork
branch and the `headRefOid` of PR #5834 are `15c9625b`. The PR itself was not
edited, and its runs on this head sit in `action_required` until a maintainer
approves them; the three `printing.yml` steps were run on this tree at the build
host instead (§5.25).

### 5.25 The three `Printing` steps on the pushed head, run at the build host (run 2026-11-10)

`Printing` is red on `467948fa` and every run on `15c9625b` sits in
`action_required`, so the question was whether the job's steps pass on the
pushed tree at all. They do, and the old red was the lock, not a lint finding.

**What the old red was.** Run `42720338000` (`Printing`, head `467948fa`) had
`Formatting` pass and `Native app and UI Clippy` fail in all three legs
(`Print (macos-latest)`, `Print (windows-latest)`, `Print (ubuntu-24.04)`); the
test steps were skipped. The failing log line is cargo's, not clippy's:
`error: cannot update the lock file …/Cargo.lock because --locked was passed to
prevent this` (exit 101 on macOS, 1 on Windows). No lint was ever printed on
that head; §5.23 records the one-line lock fix (`39f2e1a4`).

**What actually differs between the two heads.** The `Printing` steps depend on
the lock and on the crates they build, so the difference matters: `467948fa..15c9625b`
is seven commits — `a785b611` (docs), `90eaa7dc` and `3c11d775` (collab code),
`39f2e1a4` and `bf781e69` (lock), `41f67dc3` (docs) and `15c9625b` (`ci.yml`) —
not "only the lock and `ci.yml`". None of them touches `crates/ui-egui`,
`crates/doc`, `crates/layout`, `crates/engine`, `crates/docx` or
`apps/wordcraft`; the only manifest change is `apps/wordcraft-web/Cargo.toml`,
and nothing depends on that app. The two steps compile the same code at both
heads, so their result below is the result for `467948fa`'s crates too, once the
lock that failed it is fixed.

**Environment parity.** `printing.yml` runs its "Build with craft-fonts" step
before the gates and puts `CRAFT_FONTS_DIR=$GITHUB_WORKSPACE/craft-fonts` and
`CRAFT_FONTS_REQUIRED=1` into `GITHUB_ENV`, so every later step builds with the
embedded UI faces. The run below sets both the same way, against the host's
`~/craft-fonts` checkout (`b076417`, `fonts/manifest.txt` present).

**The run** (build host `agents`, `rustc 1.99.1`, clean tree, `git rev-parse
--short HEAD` on the host = `15c9625b`; 2026-11-10 15:45:55Z–15:47:46Z):

| Step (exactly as in `printing.yml`) | Result |
|---|---|
| `cargo fmt --all -- --check` | EXIT=0, no output |
| `cargo clippy -p wordcraft -p wordcraft-ui-egui --all-targets --locked -- -D warnings` | EXIT=0 (rebuild with the fonts, 11.25 s); only the three pre-existing `patch … was not used in the crate graph` warnings from upstream's patch table (§5.23) |
| `cargo test -p wordcraft -p wordcraft-ui-egui --locked` | EXIT=0 — 1293 passed, 0 failed, 4 ignored: `wordcraft` bin 78 passed/1 ignored, `wordcraft` it 8 passed, `wordcraft-ui-egui` lib 1191 passed/3 ignored, `wordcraft-ui-egui` it 16 passed |

`-D warnings` is unchanged and no `allow` was added; clippy had nothing to
report, so nothing was fixed.

**One trap worth writing down.** The same three steps without `CRAFT_FONTS_DIR`
give `fmt` EXIT=0, `clippy` EXIT=0 and `test` **EXIT=101**: one failure,
`theme::tests::interface_symbols_have_glyphs`, "symbols without a glyph:
[`。 U+3002 in tests.rs`]". That is the build, not the branch: without the fonts
the UI font set has no CJK face, and the scan picks the ideographic full stop up
from a test literal (`crates/ui-egui/src/tests.rs:110`, upstream's `b29f3a8a`)
because it reads every string in `src/`. The file is identical on `origin/main`,
and `Printing` is green on `main` `8548b33b` (run `42821245475`, 13:04Z today)
with the fonts checkout. A bare `cargo test -p wordcraft-ui-egui` on a host
without fonts therefore says nothing about CI.

**Scope.** This proves the gates pass on the pushed tree; it does not replace a
CI run. The workflows on the head this page records, `077b4056`, are `Printing`
`42862821281`, `CI` `42862822112`, `Packaging lint` `42862822199`, `Nix`
`42862822236` and `SonarCloud` `42862822414`, all created at 17:14Z and all
still `action_required`; the five runs on `15c9625b` (`Printing` `42836182389`,
`CI` `42836182540`, `Nix` `42836182675`, `SonarCloud` `42836182716`,
`Packaging lint` `42836182721`) never executed either (§5.26). Both heads need
a maintainer's approval before anything runs. `077b4056` differs from
`15c9625b` only in this page, so the run above applies to both trees.

### 5.26 The PR is ready for review; the runs on `077b4056` wait, CodeRabbit is green (run 2026-11-10)

The page still described PR #5834 as a draft on `15c9625b`, and listed the five
workflow runs of that head. Both moved. This entry records the reconciliation
and the CodeRabbit triage. The PR, the branch and CI were not touched; the fix
(`4ecab864`) and this page were local commits at that point and were pushed
later, as `af9a43a2` (§5.27).

**Draft flag.** The issue timeline has one `ready_for_review` event:
`copoka` marked the PR ready at 2026-11-10 14:57:53Z. `gh api
repos/storytold/wordcraft/pulls/5834` at 17:20Z answers `"draft": false`,
`"state": "open"`, `"mergeable": true`, `head.sha =
077b40561b426d8bd63aa43b72c82a29bce4d901`, `head.repo.full_name =
copoka/wordcraft`, `base.ref = main`. The title still ended with the word
"(draft)" as text at that point; the flag is what matters, and it was gone.
The title text itself was edited later (§5.27).

**The head moved by one docs commit.** `077b4056` is
`15c9625b` + this page's predecessor, `docs(collab): the printing gates on the
pushed head, and the push in §5.24`: `git diff --stat 15c9625b..077b4056` is
`docs/collab/STATUS.md | 93 +++…`, `1 file changed, 80 insertions(+), 13
deletions(-)`. No code and no manifest changed. At that point the local `HEAD`
and the head of `git fetch fork feature/realtime-collaboration` were both
`077b4056`; `git rev-list --count 4b0d3995..077b4056` is 77, one more than
`15c9625b` (76). Two commits sat above it at that point: the fix `4ecab864`
and this page.

**The five runs on `077b4056`.** From `gh api
"repos/storytold/wordcraft/actions/runs?head_sha=077b4056…"`:

| Workflow | Run | Status | Created |
|---|---|---|---|
| `Printing` | `42862821281` | `action_required` | 17:14:05Z |
| `CI` | `42862822112` | `action_required` | 17:14:06Z |
| `Packaging lint` | `42862822199` | `action_required` | 17:14:06Z |
| `Nix` | `42862822236` | `action_required` | 17:14:06Z |
| `SonarCloud` | `42862822414` | `action_required` | 17:14:06Z |

Nothing has run: `action_required` is the fork rule from §5.21, and the runs
on `15c9625b` did not execute either.

**CodeRabbit on `077b4056`: the check is green, and the review is triaged.**
At 17:20Z the check was still running. It completed at 17:23:31Z with
`conclusion=success` (`CodeRabbit` run `129993348649`, started 17:14:22Z), and
the review landed at 17:23:29Z: one `COMMENTED` review on `077b4056` with ten
**Major** findings, all in the review body as "outside diff range" comments —
the only diff of this head is this page, so CodeRabbit reviewed the PR's code
and could not anchor them inline. The two earlier reviews (`15c9625b`,
15:19:41Z and 15:20:49Z) and their 17 inline comments remain as they were;
their `original_commit_id` is `15c9625b` and they describe the same classes.

Each of the ten was checked against the code. None was rejected; two were fixed
in `4ecab864`, the other eight are confirmed and need their own slice.

| # | Finding (all Major) | Where | Disposition |
|---|---|---|---|
| 1 | Concurrent positional edits are applied at a stale offset; the author skips its own echo, so a replica can diverge without a resync | `crates/collab/src/room.rs:240-260` | confirmed, not fixed: the room applies primitives at server offsets and the replica applies them over its optimistic text. Needs transformation/rebase or a `baseVersion` guard that resyncs; an ADR first |
| 2 | A snapshot that cannot be read or parsed becomes an empty room, which can then be persisted over the stored file | `apps/collab-server/src/lib.rs:240-270` | confirmed, not fixed: `Store::load` returns `Option` and cannot tell "absent" from "corrupt"; `room_for` would have to reject the join instead of creating the room |
| 3 | A room with no snapshot starts blank and its snapshot replaces the open document, losing unsaved text | `apps/collab-server/src/lib.rs:240-270` | confirmed, not fixed: the protocol has no seeding step for an existing document; needs a decision on who initializes the room |
| 4 | An operation can carry another participant's `op_id`, making the victim skip the answer and then receive a cached one | `apps/collab-server/src/lib.rs:574-623` | **confirmed, fixed**: `handle_op` rejects with `Invalid` and counts a reject; test `an_operation_cannot_carry_another_clients_id` |
| 5 | When a peer's queue is full the accepted operation is dropped and the peer can stay connected, stale, if nothing follows | `apps/collab-server/src/lib.rs:791-809` | confirmed, not fixed: needs a per-peer stale flag that closes the socket on the next heartbeat, plus a test with a stalled reader |
| 6 | Any authenticated participant can register another's `client_id`; a stale connection can also remove the replacement's entry | `apps/collab-server/src/lib.rs:448-449` | confirmed, not fixed: `register` inserts by `client_id` and `leave` removes by it, so peers need a connection identity |
| 7 | A cached answer re-broadcast after a retry is applied by peers that already applied it | `apps/collab-server/src/lib.rs:598-603` | **confirmed, fixed**: the client ignores an `Applied` whose `op_id` it already confirmed; test `a_replayed_answer_for_a_confirmed_operation_is_not_applied_twice` |
| 8 | An edit whose planning or submission fails stays local while the session still reports `InSync`; a commit can omit it | `crates/ui-egui/src/collab.rs:29-47` | confirmed, not fixed: `after_command` reports the message and leaves the status; needs reconcile-and-pause |
| 9 | The connection is app-wide while documents are per window, so a connection made for one document can carry another's edits | `crates/ui-egui/src/window_ui.rs:161-173` | confirmed, not fixed: `swap_window` deliberately keeps `collab` app-wide; needs per-document connections |
| 10 | A session event for the current session replaces the client and drops its unconfirmed edits | `apps/wordcraft-web/src/web.rs:243-274` | confirmed, not fixed: `HostState::on_event(Session)` always yields `Connect` and the shell installs a new `Collab`; needs token renewal on the same client |

**The fix (`4ecab864`).** Two guards, no protocol change:

- the server refuses an operation whose `op_id.client` is not the connection's
  `client_id`, with `Invalid` and a rejects metric (§4);
- the client returns `Event::None` for an `Applied` it has already confirmed, so
  a replayed cached answer cannot apply a delta twice (§7).

**What ran** (build host `agents`, on the tree with both guards):
`cargo fmt --all -- --check` EXIT=0; `cargo clippy -p wordcraft-collab-client
-p wordcraft-collab-server --all-targets -- -D warnings` EXIT=0;
`cargo test -p wordcraft-collab-client` 18 passed; `cargo test -p
wordcraft-collab-server` 51 passed, 1 ignored (the LibreOffice commit gate).
The full `cargo xtask ci` was not run in this pass.

**What is left.** The two commits this entry describes were pushed after it was
written: `4ecab864` and `af9a43a2`, which is the PR head when the next review
landed (§5.27). They are no longer local. The eight confirmed findings above
were then taken one slice each: 1 in §5.28, 5 in §5.29, 2 in §5.30, 3 in §5.31,
6 in §5.32, 8 in §5.33 and 10 in §5.34; 9 (per-document connections) is the
only one of the ten still open and is named in §4. All of those commits are
delivered: the fork branch and the PR `headRefOid` are `50c68e3a` (§5.35). A
maintainer still has to approve the five runs on that head; the runs on
`077b4056` validate neither `4ecab864` nor this page, so their approval would
prove nothing about either.

### 5.27 The review on `af9a43a2` is about this page; the acceptance table is reconciled (run 2026-11-10)

**The push that preceded the review.** `4ecab864` and `af9a43a2` were pushed
to the fork at 20:15Z. `git rev-parse HEAD`, `git ls-remote fork
refs/heads/feature/realtime-collaboration` and `gh pr view 5834 --json
headRefOid` all answered `af9a43a2294a3de266f383a7c27f6b805e2fbd1d`; the
branch is 79 commits over `4b0d3995`. The five workflow runs on that head were
created at 20:15:37Z and every one of them is `action_required`: nothing has
executed. CodeRabbit started at 20:15:54Z and completed at 20:25:20Z with
`success`.

**The review.** `5848804525`, submitted 2026-11-10 20:25:18Z on `af9a43a2`,
state `COMMENTED`, two findings, both **Minor** and both outside the diff: the
only file this head changes is this page, so CodeRabbit could not anchor them
inline. Both concern the accuracy of this page, not the code.

**Finding 1 (`STATUS.md:300`): convergence reported as done.** Checked: the
acceptance row "Changes converge without loss or duplicates" said `done`, while
§5.26 lists finding 1 (concurrent positional edits applied at a stale offset;
the author skips its own echo, so a replica can diverge without a resync) and
finding 5 (an operation accepted while a peer's queue is full is dropped, so
the peer can stay connected and stale) as `confirmed, not fixed`. The row was
overstated and now reads **partial**, with both findings named, the two test
groups that do pass, and the plan: an ADR on transforming positional edits or
on a `baseVersion` guard that forces a resync, an ADR on a per-peer stale flag
that closes the socket on the next heartbeat, then the fixes with a stalled
reader and a two-client test. §4 carries the same limitation, so a reader who
starts there sees it too.

**Finding 2 (`STATUS.md:2605-2606`): approval of the old runs presented as
validation.** Checked: the paragraph put a maintainer's approval of the five
runs on `077b4056` first and the push of `4ecab864` and this page second. The
runs on `077b4056` were created before `4ecab864` existed, so approving them
validates neither the guard nor this page. The paragraph now says the push
happened, that approval applies to the runs on the pushed head, and that the
older runs prove nothing about either commit (§5.26 "What is left").

**The other rows of the table were checked for the same overstatement.** They
hold: two clients do edit one document (the divergence is the convergence row,
not this one); cursors and presence are covered by the panel and the socket
tests; reconnect and resync catch up from the journal and fall back to a
snapshot on a gap, and a full peer queue is a different failure, counted under
finding 5; per-user undo/redo, the viewer gate, commit and its cabinet
callback, the single-user flag, the documents and the web bridge each have
tests or a run behind them. Two changes were needed: the convergence row
(partial) and the PR row, which now names `af9a43a2` and points at this entry
for the runs. "Two real machines and the real cabinet storage" stays `not run`.

**What ran.** No code changed: `git diff --stat` of this commit is this file
only. The claims about runs and the review come from `gh api
repos/storytold/wordcraft/reviews/5848804525`, `gh api
repos/storytold/wordcraft/commits/af9a43a2/check-runs`, `gh api
"repos/storytold/wordcraft/actions/runs?head_sha=af9a43a2…"`, `gh pr view 5834`
and `git ls-remote fork`. No `cargo` command was run, because no Rust file
moved; the last full gate is the one in §3.

**What is left.** The push this entry asks for happened: the fifteen commits
after `af9a43a2` are delivered, and `50c68e3a` is the fork branch and the PR
`headRefOid` (§5.35). What remains is a maintainer's approval of the five runs
on that head, which are in `action_required` since 03:27:31Z; the runs on
`077b4056` validate neither `4ecab864` nor this page. In the code, the ADR and
the fix for finding 5 landed in §5.29 (finding 1 in §5.28), and findings 2, 3,
6, 8 and 10 followed in §5.30–§5.34; 9 (per-document connections) is the only
one of the ten still open, named in §4. The PR title ended with the word
"(draft)" as text though the flag was gone; that title was edited in this pass
and no longer says it, which touches no commit.

### 5.28 Finding 1: a positional edit that outlived its base is refused (run 2026-11-11)

**What the defect was.** The room applies every operation to the head and
computes its delta there, while the author of a primitive had already applied
it optimistically and skips its own echo (`SyncState::apply`). When another
client changed the same paragraph between the author's `baseVersion` and its
operation, the delta it received was addressed at server offsets over a text
that already held its own pending edit. The replica kept a document the server
did not have, and nothing asked it to resync: the version advanced and the peer
looked healthy (§5.26 finding 1, `crates/collab/src/room.rs:240-260`).

**The decision.** `docs/collab/adr/0001-positional-edit-guard.md` (ADR WC-0001;
repository-local, it does not consume a protocol ADR number). It compares the
three routes — a transformation table or client rebase, a `baseVersion` guard
with a forced resync, a CRDT — and picks the guard: it is the smallest change
that makes a diverged replica impossible, and it is testable with two clients
without a new merge algorithm. The ADR also states the price, which `rfc.md`
§4.4 now names instead of promising a merge: two concurrent edits inside one
paragraph can cost the later author its unconfirmed keystrokes, who continues
from the snapshot. Rebase stays the target and will supersede this ADR.

**What the code does** (`925bb0c9`, 7 files, +338/−3).

- `crates/collab/src/room.rs`: `Room::submit` refuses an offset-based primitive
  (`insert_text`, `delete_range`, `replace_range`, `split_para`, `merge_para`)
  whose `baseVersion` is behind an operation accepted since then that changed
  the text of the same paragraph, with `RejectCode::Stale` (retryable) and the
  message "the paragraph changed since this edit was made; resync". The check
  reads the journal's deltas; a `transaction` delta is scanned op by op; a
  command's delta names only the command, so the room reads its stored inverse
  and counts a `set_para_format` inverse that carries `text` (a case change or
  an enclosure, §5.10) as a change of that paragraph. The guard is per
  paragraph — an edit in another paragraph still lands — and exempts the
  author's own operations, which are applied in the same order on both sides,
  so fast typing on one base is not refused. Undo/redo requests skip it: their
  inverse is not applied optimistically at the author.
- `crates/collab-client/src/lib.rs`: `Client::on(Rejected)` drops a `stale` or
  `gone` operation from the pending queue. Those are exactly the refusals whose
  addresses or offsets cannot be replayed; before, such an operation stayed
  queued and the next reconnect resent it at the same stale offset. `lease` and
  `tokenExpired` keep their operation, because the server never examined it.
- `docs/collab/rfc.md` §4.4: the "two people type in one paragraph" row now
  states the refusal and the resync; the "stale base" row points at WC-0001.
- `docs/collab/adr/0001-positional-edit-guard.md`: the decision record above.

**Tests** (four, three of them new; each names the two-client path).

- `crates/collab-doc/src/tests.rs`:
  `a_text_edit_on_a_paragraph_another_client_changed_since_its_base_is_refused`
  — Alice's edit lands, Bob's edit on the same paragraph at `base_version` 0 is
  refused `Stale`, retryable, the text and the version are unchanged;
  `a_stale_text_edit_in_another_paragraph_still_lands` — the negative control,
  a stale edit in the second paragraph is accepted;
  `a_clients_own_earlier_edits_do_not_trip_the_guard` — three keys from one
  client on the same base all land.
- `crates/collab-client/src/tests.rs`:
  `a_refused_stale_edit_is_not_resent_after_the_resync` — the queue is empty
  after a `stale` refusal and a reconnect sends nothing, while a `lease`
  refusal keeps its operation.
- `crates/collab-app/src/tests.rs`:
  `a_stale_positional_edit_resyncs_instead_of_diverging` — Alice's edit lands,
  Bob's optimistic edit in the same paragraph is refused, Bob takes the room's
  snapshot (`join(None, false)` → `set_document`, the path the transport takes
  on `Snapshot`), and both replicas hold the same document JSON. Before the
  resync Bob's replica says `B` where the room says `A`; after it, `A` on both.
- The two refusal tests were checked against a tree with the guard call removed
  on the build host: both fail there (`Applied {version: 2, conflict: true}`
  instead of the refusal), so they pin the fix.

**What ran** (build host `agents`, `scripts/remote-build.sh`, from this tree):

- `cargo fmt --all -- --check` → **EXIT=0**.
- `cargo clippy -p wordcraft-collab -p wordcraft-collab-client
  -p wordcraft-collab-doc -p wordcraft-collab-app -p wordcraft-collab-server
  --all-targets -- -D warnings` → **EXIT=0**.
- `cargo test` over those five crates → **284 passed, 0 failed, 1 ignored**
  (`collab` 59, `collab-doc` 116, `collab-client` 19, `collab-app` 39,
  `collab-server` 51 passed / 1 ignored — the LibreOffice commit gate).
  The slice added 5 tests: `collab-doc` 113 → 116, `collab-client` 18 → 19,
  `collab-app` 38 → 39.
- `.github/workflows/` untouched, no manifest or lock change, no
  `main`-touching step.

**Local head.** `925bb0c9` (the fix, its tests, the ADR and the RFC row); this
page is one more commit on top of it and the only file it changes. `git log -1`
and `git show --stat` for the fix are the ones the commit printed:

| | |
|---|---|
| head | `925bb0c9` — `collab: refuse a positional edit whose base no longer matches its paragraph` |
| files | `crates/collab/src/room.rs` +62/−2, `crates/collab-client/src/lib.rs` +8, `crates/collab-client/src/tests.rs` +24, `crates/collab-doc/src/tests.rs` +57, `crates/collab-app/src/tests.rs` +43, `docs/collab/adr/0001-positional-edit-guard.md` +143, `docs/collab/rfc.md` +1/−1 |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The push happened: this commit is in `50c68e3a`, the fork
branch and the PR head (§5.35), and a maintainer's approval of the five runs on
that head is still outstanding. In the code, finding 5 is next (§4): a per-peer
stale flag that closes the socket when an accepted operation was dropped from a
full queue, with a stalled-reader test; that is what keeps the convergence
criterion at **partial** with finding 1 closed. Findings 2, 3, 6, 8, 9 and 10
follow, each with its own test. The rebase for text primitives stays the design
target and will supersede ADR WC-0001.

### 5.29 Finding 5: a peer that missed an accepted operation is closed, not left stale (run 2026-11-11)

**What the defect was.** Each peer has a bounded outbox and `broadcast` hands it
an accepted operation with `try_send`, so one slow client cannot grow the server.
When the outbox was full the frame was dropped silently and the connection stayed
open: the client learns about a gap only from a later frame, so with nothing
following it kept a document the room had moved past and still reported itself
in sync. A full outbox means the peer stopped reading, which is the same failure
as the stalled reader (§5.26 finding 5, `apps/collab-server/src/lib.rs`).

**The decision.** `docs/collab/adr/0002-stale-peer-close.md` (ADR WC-0002;
repository-local, it does not consume a protocol ADR number). It compares
relying on the next frame, a stale flag plus a closed connection, an unbounded
or blocking outbox, and dropping the peer from the room while keeping its socket;
it picks the flag. The ADR also states the addition the flag alone cannot
provide: a flag read by a loop that is inside the blocked write is never read,
and that write is blocked precisely because the peer stopped reading.

**What the code does** (`264afd18`, 6 files, +533/−96).

- `apps/collab-server/src/lib.rs`: `Peer` carries a `Stale` (an `AtomicBool` plus
  a `tokio::sync::Notify`). A document frame a peer's outbox cannot take marks
  the peer stale, counts `wordcraft_collab_stale_peers_total` once per peer and
  logs a warning without document content; presence and `left` stay droppable
  and never mark a peer (ADR-0006). The connection loop closes a stale peer
  before its next write, wakes out of a write that is stuck on that peer through
  the notification, and checks the flag on every heartbeat — so a drop noticed
  while the loop is busy with an inbound frame still ends the connection on the
  next tick. `send` is bounded by the heartbeat period, as are the ping and the
  join frames, so a socket that stops taking bytes without ever filling the
  outbox is closed too. Closing runs the existing `leave`, so the other peers
  get `left` and the client reconnects with `lastVersion` and takes the journal
  tail or a snapshot.
- `Config` gains `heartbeat` (`PRESENCE_TTL / 3`, the write bound and the close
  interval) and `peer_queue` (512, `PEER_QUEUE`), so the tests can reach the
  overflow without a megabyte-sized socket buffer instead of changing the
  production bound.
- The frame handlers take one `Conn` (state, room, claims, role, client id,
  write bound) instead of seven arguments, which is what the write bound would
  otherwise have grown into.
- `docs/collab/rfc.md` §4.4 has a row for a peer that stops reading; §5 says
  presence never marks a peer stale. `server.md` names the outbox bound and the
  write bound; `production.md` lists the new counter.

**Tests** (two new; the third is the path the close relies on).

- `apps/collab-server/src/tests.rs`:
  `a_peer_that_stops_reading_is_closed_and_resyncs_instead_of_staying_stale` —
  Alice pushes 64 KiB edits while Bob's socket is never read; the server marks
  Bob stale (`stale_peers_total == 1`), Alice sees `left{bob}`, Bob's socket
  ends, and Bob's reconnect with `lastVersion: 0` reports the room's head and
  replays the journal tail (or a snapshot), so it catches up instead of staying
  stale.
- `a_full_outbox_marks_the_peer_stale_once` — the flag without a socket: the
  first frame fits a one-slot outbox, the second is dropped and marks the peer,
  a third is the same event (counted once).
- `a_reconnecting_client_catches_up_from_the_journal` still passes.

**The tests fail without the fix.** On the build host, with the flag never set,
`a_peer_that_stops_reading…` fails on the counter (`0 != 1`); with the write
bound removed as well, the peer is never closed and the test fails at
`the stalled peer was closed instead of staying connected (head version 400)` —
the finding reproduced.

**What ran** (build host `agents`, `scripts/remote-build.sh`, from this tree):

- `cargo fmt --all -- --check` → **EXIT=0**.
- `cargo clippy -p wordcraft-collab-server -p wordcraft-collab-client
  --all-targets -- -D warnings` → **EXIT=0**.
- `cargo test` over `collab`, `collab-server`, `collab-client`, `collab-doc`,
  `collab-app` → **286 passed, 0 failed, 1 ignored** (`collab` 59, `collab-doc`
  116, `collab-client` 19, `collab-app` 39, `collab-server` 53 passed / 1
  ignored — the LibreOffice commit gate). The slice added 2 tests:
  `collab-server` 51 → 53.
- `.github/workflows/` untouched, no manifest or lock change, no
  `main`-touching step.

**Local head.** `264afd18` — `collab: close a peer whose outbox overflowed
instead of leaving it stale`; this page is one more commit on top of it and the
only file it changes.

| | |
|---|---|
| head | `264afd18` |
| files | `apps/collab-server/src/lib.rs` +232/−90, `apps/collab-server/src/tests.rs` +151/−4, `docs/collab/adr/0002-stale-peer-close.md` +139, `docs/collab/server.md` +7/−1, `docs/collab/rfc.md` +3/−1, `docs/collab/production.md` +1 |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The commit is delivered in `50c68e3a` (§5.35); a maintainer's
approval of the runs on that head is still outstanding. In the code, findings
2, 3, 6, 8, 9 and 10 remain (§4), each with its own slice and test; finding 2 (a
corrupt snapshot becomes an empty room) is the next data-safety one. Both
convergence findings are now closed, so §2's criterion reads **done** with the
price the two ADRs name; the six remaining findings are separate failure modes
and are not hidden by that row.

### 5.30 Finding 2: a snapshot that cannot be opened refuses the document (run 2026-11-11)

**What the defect was.** `AppState::room_for` created a room on the first join.
`Store::load` returned `Option<Snapshot>`, and `None` meant both "no snapshot
was ever written" and every way a stored one can fail: an unreadable
`model.json`, a missing or unparsable `meta`, a model that is not a `Document`,
a model over the size bound. All of them fell into one `unwrap_or_else` that
built the room from `Document::new()` (§5.26 finding 2,
`apps/collab-server/src/lib.rs:240-270`).

The blank room is a normal room. The first operation makes it dirty, `persist`
writes it after 500 operations or on `leave`, and `persist_all` writes it on
SIGTERM. `save` rotates the previous pair to `prev/` once, so the damaged file
survives exactly one more snapshot — after that it is gone and the document is
an empty page nobody asked for.

**The decision.** `docs/collab/adr/0003-corrupt-snapshot-refused.md` (ADR
WC-0003; repository-local, it does not consume a protocol ADR number). It
compares keeping the fallback with deeper rotation, quarantining the file and
starting blank, refusing to start the server, and refusing the document; it
picks the last. The store is the only place that can see the difference, so the
store owns it: `Store::load` answers `Ok(None)` (never written), `Ok(Some)`
(opens) or `Err(Corrupt { file, reason })` (there and cannot be opened). The
room table gets no entry on `Err`, so nothing can persist, rotate or commit over
the file.

**What the code does** (`apps/collab-server`, plus the ADR and four docs:
+536/−79 in seven files).

- `src/store.rs`: `load` returns `Result<Option<Snapshot>, Corrupt>`. `model.json`
  absent is `Ok(None)`; unreadable, over `MAX_MODEL` (`64 MiB`), or not a
  `wordcraft_doc::Document` is `Err` on `model.json`. `meta` is required (it is
  written last, so a model without it never committed), `version` is required
  and every known key must be a number — starting at 0 would move the head
  backwards; `epoch` may be absent (an older snapshot) and starts at 0. A
  `cabinet_version` without its etag (or the reverse) is `Err`: dropping the
  etag would drop the conflict check that stops the next commit from overwriting
  an external edit. `dedup.json` and `source.docx` stay forgiving, as in §5.5:
  a damaged window costs a duplicate answer, a missing source costs original
  markup, and neither is text.
- `src/lib.rs`: `room_for` returns `Result<Arc<Mutex<RoomState>>, Corrupt>` and
  does not insert a room on `Err`; `refuse_corrupt` answers the join with
  `Rejected { code: invalid, retryable: false }` and closes the socket, logging
  the file, the reason and the directory an operator has to look at, without
  document content. `Metrics` gained `corrupt_snapshots_total`. `RoomState::new`
  removes the field duplication the two room constructors had.
- `docs/collab/server.md` (the three states and the recovery path),
  `production.md` (the counter and what to do when it grows), `rfc.md` §4.4 and
  §4.6 (one row and one bullet; the wire is unchanged).

**Tests** (five new; four pin the distinction, one the join).

- `src/store.rs`: `a_damaged_model_is_corrupt_not_absent` (a truncated
  `model.json` is `Err` on `model.json`), `a_snapshot_without_its_metadata_is_corrupt`
  (the interrupted-write shape is `Err` on `meta`), `metadata_that_is_not_a_number_is_corrupt`
  (`epoch=now`, `version=five`), `half_a_cabinet_marker_is_corrupt`, and
  `a_missing_snapshot_is_none` (the absent case, asserted against them).
- `src/tests.rs`: `a_corrupt_snapshot_refuses_the_join_and_is_not_overwritten` —
  with a truncated model and a `meta` on disk, a real client gets
  `Rejected { invalid, retryable: false }`, `room_count()` is 0,
  `corrupt_snapshots_total` is 1, `persist_all()` leaves `model.json` and `meta`
  byte-for-byte unchanged, and a document with nothing stored still opens a room
  at version 0.
- `the_previous_snapshot_is_kept` used `b"one"`/`b"two"` as models; it now uses
  real documents, because the store refuses to hand out anything else.
- **The tests fail without the fix.** On the build host, with `room_for` falling
  back to an empty room, the join test fails at the first `match` — the client
  gets `Welcome { version: 0 }`, not a refusal — and, with that assertion
  bypassed, at
  `no empty room may stand in for the document` (`1 != 0`) and then at
  `the damaged model is left for an operator`: `persist_all()` writes the empty
  document JSON over the truncated bytes. The finding reproduced.

**What ran** (build host `agents`, `scripts/remote-build.sh`, from this tree):

- `cargo fmt --all -- --check` → **EXIT=0**.
- `cargo clippy -p wordcraft-collab-server --all-targets -- -D warnings` →
  **EXIT=0** (the first attempt failed on `cognitive_complexity` (31/30) in
  `handle_socket`; the refusal moved to `refuse_corrupt`).
- `cargo test` over `collab`, `collab-server`, `collab-client`, `collab-doc`,
  `collab-app` → **291 passed, 0 failed, 1 ignored** (`collab` 59, `collab-doc`
  116, `collab-client` 19, `collab-app` 39, `collab-server` 58 passed / 1
  ignored — the LibreOffice commit gate). The slice added 5 tests:
  `collab-server` 53 → 58.
- `.github/workflows/` untouched, no manifest or lock change, no
  `main`-touching step.

**Local head.** The fix, its tests and the ADR are `bcca9bce` — `collab: refuse a
document whose snapshot cannot be opened`; this page is one more commit on top of
it and the only file it changes.

| | |
|---|---|
| head | `bcca9bce` — `collab: refuse a document whose snapshot cannot be opened` |
| files | `apps/collab-server/src/lib.rs` +95/−46, `src/store.rs` +184/−23, `src/tests.rs` +61/−10, `docs/collab/adr/0003-corrupt-snapshot-refused.md` +166, `docs/collab/server.md` +16, `production.md` +7, `rfc.md` +7 (7 files, +536/−79) |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The commit is delivered in `50c68e3a` (§5.35); a maintainer's
approval of the runs on that head is still outstanding. Two follow-ups this
slice names and does not take:

- **The client reconnects in a loop on a refused join.** The transport
  reconnects on any closed socket with a 250 ms backoff, and a join refusal
  closes the socket, so a damaged document produces a reconnect every 250 ms
  (each one counted). The refusal is not retryable and the app shows `Failed`,
  but nothing tells the transport that a non-retryable refusal before `Welcome`
  is final. It applies to the full-room and lease refusals too; it is a client
  slice of its own.
- **`prev/` is a one-step rollback, not a backup.** With the join refused the
  pair is left alone, so `prev/` still holds the snapshot before the damage was
  noticed — but nothing rotates or copies it on its own. Restore is an operator
  action (`server.md`), and a real backup is the deployment's, not the store's.

In the code, findings 3, 6, 8, 9 and 10 remain (§4), each with its own slice and
test; finding 3 (a room with no snapshot replaces the open document) is the next
data-safety one and now sits on the `Ok(None)` branch this slice made explicit.

### 5.31 Finding 3: a room without a snapshot is seeded by its first writer (run 2026-11-11)

**What the defect was.** `AppState::room_for` created a room on the first join,
and for a document the store had never snapshotted (`Ok(None)`, the branch
§5.30 made explicit) it built one from `Document::new()`. `Room::join` answers a
client without `lastVersion` with a snapshot, so that client received the empty
document and `Session::set_document` replaced the document it had open: the text
on screen, including edits it had not committed, was gone from its window, the
window then said `InSync`, and the next commit wrote an empty `.docx` over the
cabinet's copy. The protocol had no seeding step, so `last_version: null` meant
both "I have nothing" and "I have the document, tell me what the room holds"
(§5.26 finding 3, `apps/collab-server/src/lib.rs:240-270`).

**The decision.** `docs/collab/adr/0004-room-without-snapshot-seeded.md`
(ADR WC-0004; repository-local, it does not consume a protocol ADR number). It
compares keeping the blank room and hiding the divergence in the client,
refusing every join to an unsnapshotted room, having the server fetch the file
from the cabinet, and seeding from the first writer's copy; it picks the last.
The client that has the document open is the only party that can seed the room,
and the server's rule becomes one sentence: **a room is created from state — a
stored snapshot or a client's copy — and never from a default.**

**What the code does** (+890/−81 in sixteen files).

- `crates/collab/src/wire.rs`: `ClientMsg::Auth` gains `seed: bool`
  (`#[serde(default)]`, so an older client's auth still parses) and
  `ClientMsg::Seed { bytes, last }` carries the document in chunks. A `seed`
  frame outside the handshake is refused `invalid`.
- `apps/collab-server/src/lib.rs`: `authenticate` returns a `Handshake` (it was
  a four-tuple) and, after the token is verified, `read_seed` accumulates the
  stream bounded by `MAX_MODEL` and `AUTH_TIMEOUT` per frame. `room_for` takes
  the seed and the role and answers `(room, seeded)` or a `Refusal`: `Corrupt`
  (ADR WC-0003), `Unseeded` (nothing stored, nothing offered, or a role that may
  not write), `BadSeed` (the bytes are not a document). `read_model` is now the
  single parser for a stored model and an offered one. `send_join` takes
  `seeded` and sends no snapshot back to the client whose copy became the room:
  `welcome.resync` is false and the client keeps its document, selection and
  undo history. `Metrics` gained `unseeded_joins_total`, and `refuse_join`
  replaced the corruption-only refusal helper.
- Clients: `Collab::connect` (desktop `crates/collab-app`, web
  `crates/collab-web`) takes the open `Document` and keeps its `serde_json`;
  `collab-ws` and `collab-web` send `auth{seed}` plus
  `wordcraft_collab_client::seed_frames` while nothing is accepted
  (`version == 0`), which is exactly the window in which the server may need
  them. `crates/ui-egui/src/collab.rs` and `apps/wordcraft-web/src/web.rs` pass
  `app.session.doc`.
- Clients offer the document as it was when the session started, so the
  unconfirmed queue that follows rebuilds the current text: the replay is not
  applied twice. A room that already has state ignores the seed and answers with
  its snapshot, so a stale copy can never overwrite it.
- `docs/collab/server.md` (the handshake, the three store states, the frame
  bounds), `production.md` (the counter and what to do when it grows), `rfc.md`
  §4.2/§4.5/§4.6 (the two message rows and two paragraphs) and `host-bridge.md`
  (the host opens the session after the document is in the app, because the seed
  is whatever the app holds).

**Tests** (nine new: five pin the server rule, two the wire, one the client core,
one the transport).

- `apps/collab-server/src/tests.rs`:
  `a_client_with_an_open_document_seeds_the_room_it_joins` (the seeding client
  gets `welcome{version: 0, resync: false}` and no snapshot; a second client's
  snapshot contains the first client's text; an edit at that paragraph's offset
  lands at version 1), `a_join_without_a_seed_is_refused_when_nothing_is_stored`
  (`invalid`, not retryable, no room, the counter counts, and a viewer with a
  copy is refused the same way), `a_new_document_still_opens_a_room`,
  `a_seed_does_not_replace_a_stored_snapshot` (the restored room answers with
  its own document and the offered copy never reaches it) and
  `a_seed_that_is_not_a_document_is_refused`. The connect helpers now offer an
  empty document, which is the room the old tests used to get; `open_room` and
  `a_viewer_cannot_write` were adjusted for the two states "stored" and "a
  viewer never creates a room".
- `crates/collab/src/wire.rs`: `an_older_auth_without_a_seed_field_parses`,
  `a_seed_chunk_round_trips_through_json`, and the auth carries `seed` when it
  says so.
- `crates/collab-client/src/tests.rs`:
  `seed_frames_split_the_document_and_mark_exactly_one_last_chunk` (the parts
  rebuild the model, one `last`, and an empty document still sends its frame).
- `crates/collab-ws/tests/reconnect.rs`:
  `the_first_connection_offers_the_open_document` — the transport announces the
  seed and the fake server rebuilds the document byte for byte.
- **The tests fail without the fix** (run at the build host with `room_for` back
  on `Document::new()` for `Ok(None)`). The seed test fails at its first
  assertion — the seeding client gets `welcome{resync: true}` and a snapshot
  instead of `resync: false`; with that assertion bypassed it fails at
  `the room kept the document the first client had open` and prints the empty
  model (`"text":""`, no runs) the second client received. The refusal test
  fails at its first `match`: `expected the join to be refused, got Welcome { …
  version: 0, … resync: true }` — the blank room and its snapshot, which is the
  finding. Both patches were reverted; the tree is clean.

**What ran** (build host `agents`, `scripts/remote-build.sh`, from this tree at
`bf632378`):

- `cargo fmt --all -- --check` → **EXIT=0**.
- `cargo clippy -p wordcraft-collab-server --all-targets -- -D warnings` →
  **EXIT=0** (`collab-client`, `collab-ws` and `collab-web` with all targets are
  also clean under `-D warnings`, which the changed call sites need).
- `cargo test` over `collab`, `collab-server`, `collab-client`, `collab-doc`,
  `collab-app` → **299 passed, 0 failed, 1 ignored** (`collab` 61, `collab-doc`
  116, `collab-client` 20, `collab-app` 39, `collab-server` 63 passed / 1
  ignored — the LibreOffice commit gate). The slice added 9 tests:
  `collab-server` 58 → 63, `collab` 59 → 61, `collab-client` 19 → 20, and
  `collab-ws`'s `reconnect` test 1 → 2 (2 passed, outside the five-crate set).
- `cargo xtask wasm` → `wasm32-unknown-unknown check: ok (22 crates)`; the wasm
  build of the web app was checked on its own as well:
  `cargo check -p wordcraft-web --target wasm32-unknown-unknown` → **EXIT=0**,
  because the app binary is not in the xtask set and it carries one changed call
  site.
- `.github/workflows/` touched: no; manifest or lock changed: no; nothing that
  touches `main`.

**Local head.** The fix, its tests and the ADR are `bf632378` — `collab: open a
room from the document the first writer has open`; this page is one more commit
on top of it and the only file it changes.

| | |
|---|---|
| head | `bf632378` — `collab: open a room from the document the first writer has open` |
| files | `apps/collab-server/src/lib.rs` +174/−43, `src/tests.rs` +248/−18, `crates/collab/src/wire.rs` +39/−1, `crates/collab-client/src/lib.rs` +30/−3, `src/tests.rs` +29, `crates/collab-ws/src/lib.rs` +22/−4, `tests/reconnect.rs` +45/−1, `crates/collab-app/src/lib.rs` +6/−2, `crates/collab-web/src/lib.rs` +28/−3, `crates/ui-egui/src/collab.rs` +1/−1, `apps/wordcraft-web/src/web.rs` +1/−1, `docs/collab/adr/0004-room-without-snapshot-seeded.md` +197, `server.md` +32/−3, `rfc.md` +21/−1, `production.md` +10, `host-bridge.md` +7 (16 files, +890/−81) |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The commit is delivered in `50c68e3a` (§5.35); a maintainer's
approval of the runs on that head is still outstanding. Three follow-ups this
slice names and does not take:

- **The seed carries no source `.docx`.** The collab session does not hold the
  host's file bytes, so a room seeded from a model has no `source.docx` until
  its first commit, and a commit before that writes the modeled form of
  unchanged paragraphs with the ADR-0011 warning. Text is not affected; the
  follow-up is a second seed stream for the source, fed by the host bridge.
- **The seed carries no cabinet `version`/`etag`,** so a seeded room's first
  commit is still unconditional (§4). The clients that can seed do not know the
  etag; the host does. Until the seed carries it, the existing limitation
  stands, and it is now the only unprotected window a seeded room has.
- **A room whose snapshot was lost while clients were connected stays refused.**
  Those clients have `version > 0` and therefore do not seed; reopening the
  document seeds a new room. That is loud instead of silent, and it shares the
  §5.30 follow-up: the client reconnects in a loop on a non-retryable refusal
  (`invalid`), so the window keeps counting `unseeded_joins_total` until someone
  closes it or reopens the document.

In the code, findings 6, 8, 9 and 10 of §5.26 remain (§4), each with its own
slice and test. Finding 6 (a participant can register another's `client_id`) is
the next protocol-shaped one; findings 8–10 are client-side state.

### 5.32 Finding 6: a `client_id` belongs to the connection that registered it (run 2026-11-11)

**What the defect was.** A room keyed its peers by the `client_id` a client puts
in its auth frame, and both ends of that map were keyed by the id alone:
`register` did `peers.insert(client_id, …)` and `leave(client_id)` did
`peers.remove(client_id)` (§5.26 finding 6,
`apps/collab-server/src/lib.rs:448-449` at `af9a43a2`). Two consequences:

1. **A participant could take another's id.** Any authenticated connection that
   sent `client_id: "alice"` replaced Alice's entry. Inside one cabinet session
   the lease does not stop it, and as `owner` the lease does not stop it either.
   Alice kept a socket that received nothing from the room while the participant
   list showed the intruder under her id; in the other order her `leave` removed
   the intruder's entry and the room lost a live connection it believed had
   left.
2. **A reconnect raced its own predecessor.** The reconnect path ADR WC-0002 and
   §5.31 rely on is: a peer is closed, the client reconnects with the same
   `client_id` and `lastVersion`. Two sockets are served by two tasks, so the
   server can accept the new connection before the old task reaches `leave`;
   then `register(new)` → `leave(old)` removes the new entry. That client was in
   the room for operations but was not a participant, received no broadcasts and
   did not count for the lease.

The id has to stay the client's: `opId = (clientId, seq)` is the retry key across
a reconnect (ADR-0003), and the unconfirmed queue is keyed by it.

**The decision.** `docs/collab/adr/0005-client-id-belongs-to-a-connection.md`
(ADR WC-0005; repository-local, it does not consume a protocol ADR number). It
compares refusing every duplicate registration (rejected: the reconnecting client
is refused while the server still holds the socket it cannot notice is gone, and
`invalid` is not retryable), having the server assign the id (rejected: a wire
change in a frozen protocol, and the queue and pending `opId`s would have to be
rewritten), and comparing the peer's outbox channel instead of a counter
(rejected as the mechanism: it says nothing about who the connection is, and the
refusal rule still needs the identity on the peer). The rules it picks:

- **ownership comes from the token, never from the frame** — the frame picks the
  id, the JWT says who is claiming it;
- **takeover requires the same `sub` and the same `sid`** — the same client
  reconnecting; the old connection is retired with a `close` frame;
- **a foreign claim is refused `invalid`, not retryable**, and counted;
- **removal is by connection, not by id** — a retired connection's `leave` is a
  no-op: it broadcasts no `left` and does not treat the room as empty;
- **no wire change.**

**What the code does** (+518/−88 in five files).

- `apps/collab-server/src/lib.rs`: `Peer` gains `conn` (a server counter), `sub`
  and `superseded`. `register` takes the `Claims` and answers a `Registration`
  (`plan`, `peers`, `conn`, the two signals): a second registration of a taken id
  is refused with `client_id_conflicts_total`, or, for the same `sub`+`sid`, is a
  takeover that marks the old peer superseded. `leave` takes the connection id
  and removes the entry only when it matches. The signal type ADR WC-0002
  introduced as `Stale` is now `Retire` (a flag, a `Notify`, and a wait), used
  per connection for both events. The connection loop moved out of
  `handle_socket` into `Session::run`: a fourth `select!` arm pushed the function
  over the crate's `clippy::cognitive_complexity` bound (31/30), and the
  handshake is the part that belongs in `handle_socket`.
- `docs/collab/server.md`: the ownership rule, under the session flow.
  `production.md`: the counter and what to do when it grows.
- No manifest, lock or workflow change.

**Tests** (three new; `collab-server` 63 → 66).

- `another_participant_cannot_register_a_taken_client_id` — Alice joins; a
  connection that authenticates as Bob and registers `alice` is answered
  `rejected{invalid, retryable: false}`, its socket ends, the room still holds one
  peer whose `sub` is `alice`, Alice's next operation is applied, and
  `client_id_conflicts_total == 1`.
- `a_reconnecting_client_takes_its_client_id_over_and_the_old_leave_keeps_the_new_peer`
  — Carol observes; Alice joins, then reconnects with the same id and identity.
  The old socket receives `close`, Carol receives no `left` (the old `leave` is a
  no-op), the room still lists two peers, and the replacement's edit is applied
  and broadcast to Carol.
- `a_taken_client_id_is_only_removed_by_the_connection_that_registered_it` — the
  same orderings without a socket: a foreign claim is `Err(invalid)`; the same
  identity takes over and retires the old signal; `leave` with the old `conn`
  keeps the replacement; `leave` with the new `conn` removes it.

**The tests fail without the fix** (run at the build host, the tree at
`c1582864`, one guard reverted at a time; both patches reverted afterwards, tree
clean).

- With the identity check disabled in `register`, the claim test fails at its
  first `match`: `expected a claim on a taken client id to be refused, got
  Welcome { session_id: "session-1", role: "editor", version: 0, epoch: 0,
  participants: [Participant { client_id: "alice", … }], resync: true }` — the
  room accepted the intruder, and the entry under `alice` is now the intruder's.
- With `leave` back on `guard.peers.contains_key(client_id)`, the direct test
  fails at `the old leave kept the replacement` (the entry holds the new
  connection's `conn`) and the socket test fails at its `left` assertion — Carol
  is told the participant left and the room drops a live connection.

**What ran** (build host `agents`, `scripts/remote-build.sh`, from this tree at
`c1582864`):

- `cargo fmt --all -- --check` → **EXIT=0**.
- `cargo clippy -p wordcraft-collab-server -p wordcraft-collab-client
  --all-targets -- -D warnings` → **EXIT=0**.
- `cargo test` over `collab`, `collab-server`, `collab-client`, `collab-doc`,
  `collab-app` → **302 passed, 0 failed, 1 ignored** (`collab` 61, `collab-doc`
  116, `collab-client` 20, `collab-app` 39, `collab-server` 66 passed / 1
  ignored — the LibreOffice commit gate). The slice added exactly the three
  `collab-server` tests above; the other four crates are unchanged and green.

**Local head.**

| | |
|---|---|
| head | `c1582864` — `collab: a client id belongs to the connection that registered it` |
| files | `apps/collab-server/src/lib.rs` +180/−81, `src/tests.rs` +161/−7, `docs/collab/adr/0005-client-id-belongs-to-a-connection.md` +158, `server.md` +10, `production.md` +9 (five files, +518/−88) |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The commit is delivered in `50c68e3a` (§5.35); a maintainer's
approval of the runs on that head is still outstanding. Two things this slice
names and does not take:

- **The ownership rule assumes `sid` is stable for the life of a `client_id`.**
  That is how the cabinet issues tokens (ADR-0008) and how the desktop transport
  reconnects (it keeps its token and its id, `collab-ws`). A client that renewed
  its token *and* changed `sid` while keeping the id would be refused `invalid`;
  the web bridge avoids it by deriving a fresh `client_id` per connection.
- **A takeover by the same identity is neither counted nor logged as a claim.**
  It is a reconnect, but repeated takeovers of one id mean a client reconnecting
  in a loop, which is a different problem from an intruder; the existing
  `connections_total` and `stale_peers_total` are where that shows.

In the code, findings 8, 9 and 10 of §5.26 remain (§4), each with its own slice
and test; they are client-side state, not the protocol. Finding 6 is closed here.

### 5.33 Finding 8: an edit that did not reach the room pauses the session (run 2026-11-11)

**What the defect was.** A local command runs in this window before the room
hears about it: `before_command` clones the document, the engine runs the
command, and `after_command` plans the difference and submits the operations
(`crates/ui-egui/src/collab.rs:29-47` at `af9a43a2`, §5.26 finding 8). Both of
the last two steps can fail:

1. **Planning.** `SyncState::plan` answers `Err` when the diff cannot address the
   document (`DiffError::MissingId` — a paragraph the server issued no id for)
   or when the command fallback has no addressable selection (`"{command} needs
   a session id for its selection"`).
2. **Submission.** `Handle::submit` answers `Err("the session task is gone")`
   when the transport task has ended; the browser's `send` answers `Err` when the
   socket is not open. With several planned operations the failure can also land
   after the first ones were accepted, so the room holds part of the edit.

The old `after_command` showed the string in the status bar and changed nothing
else. `Status` stayed `InSync`, so `can_write` and `can_commit` stayed true, the
panel said "synchronized", and the next commit asked the room to build the file
**at the room's version**, which does not contain the edit. `TASK.md` §2's
convergence criterion was reported as met while a writer's paragraphs could be
left out of the stored `.docx`.

**The decision.** `docs/collab/adr/0006-diverged-session-pauses.md` (ADR
WC-0006; repository-local). It rejects three alternatives: keeping the status and
only showing the message (that is the defect); restoring the pre-command document
(the engine has already run the command, and a command's inverse is not
universal — §4); keeping the edit and retrying later (the offline queue that does
not exist). The rules it picks:

- **`Status::Diverged` is a status of its own**, not `Resyncing`: "this document
  holds an edit the room does not have" is a different fact from "a handshake is
  in progress". Both gates require `InSync`, so writing and committing stop
  without a second flag.
- **Both failures are one outcome.** `Pause`, `plan_command` and `submit_plans`
  in `wordcraft-collab-app` turn either stage into the same status and the same
  message ("…a commit is blocked until the session is synchronized"); the
  desktop and the web session use the same two helpers, so the transports cannot
  drift apart here.
- **A pause asks for the room's state, and only a snapshot ends it.** The
  desktop sends `Handle::resync`, the web session `ClientMsg::Resync`. A
  `welcome` no longer ends it: a reconnect hands back the handshake, not the
  document, so a diverged session stays paused across it and asks for the
  snapshot again.
- **A transport that cannot even ask is not left paused.** If the resync request
  fails too, the desktop reports `Failed(…)` (the task is gone; nothing will
  repair it) and the web reports `Offline` (the socket is not open; the transport
  reconnects and rejoins). Neither hides the reason: `last_error` keeps it.
- **The commit is refused with the reason.** The owner keeps a disabled Commit
  button whose hover text says why, `request_commit` reports the same reason when
  a menu or a shortcut calls it, and the edit gate answers with the divergence
  sentence instead of "reconnecting". A read-only role can no longer reach the
  host-owned commit path through this branch either. The cabinet's
  `session-state` reports `pending` while diverged (the contract has no word for
  this state and a snapshot is on its way); the cabinet gets no detail by
  design.

**The change.** Five files, +385/−33:

- `crates/collab-app/src/lib.rs` — `Status::Diverged`; `Pause` with its one
  message; `plan_command` and `submit_plans`; `Collab::after_command` calls them
  and pauses on either failure; `Collab::pause` keeps the reason, sets the
  status, and asks for the snapshot; the welcome handler keeps `Diverged` and
  asks again.
- `crates/collab-web/src/lib.rs` — the same path over the browser socket:
  `deliver` (send when joined, queue before `welcome`), `pause`, the same welcome
  rule; `host_status` maps `Diverged` to `pending`.
- `crates/ui-egui/src/collab.rs` — the panel label "out of sync — an edit did not
  reach the room", a disabled Commit button with the reason on hover,
  `request_commit` reporting the reason instead of doing nothing, and
  `pause_message` used by the edit gate.
- `crates/collab-app/src/tests.rs` — the three tests below.
- `docs/collab/adr/0006-diverged-session-pauses.md` — the record.

**What ran** (build host `agents`, on this tree):

- `cargo fmt --all -- --check` EXIT=0;
- `cargo clippy -p wordcraft-collab -p wordcraft-collab-app -p wordcraft-collab-web
  -p wordcraft-collab-client -p wordcraft-collab-doc -p wordcraft-collab-server
  --all-targets -- -D warnings` EXIT=0;
- `cargo check -p wordcraft-ui-egui --all-targets` EXIT=0;
- tests: `wordcraft-collab` 61 passed, `wordcraft-collab-server` 66 passed and 1
  ignored (the LibreOffice commit gate), `wordcraft-collab-client` 20,
  `wordcraft-collab-doc` 116, `wordcraft-collab-app` 42 (39 before this slice; the
  three new ones are the failure paths and the guard against a false pause),
  `wordcraft-collab-web` 5 — **310 passed, 0 failed, 1 ignored** in the six
  crates. The two new failure-path tests fail with the `Pause`'s status set back
  to `InSync`, which is the pre-fix behaviour; the patch was reverted after the
  check.
- `wordcraft-ui-egui`: 1191 passed, 3 ignored, 0 failed, with
  `CRAFT_I18N_DIR`/`CRAFT_FONTS_DIR` from the host. Without them the suite's
  `theme::tests::interface_symbols_have_glyphs` fails on the missing font
  sources — the same environmental dependency the earlier passes record (§3,
  §5.22). It passes with them, unmodified.

**Local head.**

| | |
|---|---|
| head | `3b05f844` — `collab: an edit that did not reach the room pauses the session` |
| files | `crates/collab-app/src/lib.rs` +90/−5, `src/tests.rs` +56, `crates/collab-web/src/lib.rs` +49/−11, `crates/ui-egui/src/collab.rs` +55/−17, `docs/collab/adr/0006-diverged-session-pauses.md` +135 (five files, +385/−33), plus this page |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The commit is delivered in `50c68e3a` (§5.35); a maintainer's
approval of the runs on that head is still outstanding. Two things this slice
names and does not take:

- **The price is the one ADR WC-0002 already names.** The resync continues from
  the room's snapshot, so an edit that never reached the room is dropped locally
  rather than merged. Keeping it needs the offline queue §4 has always named as
  absent; the pause is the safe half of that queue.
- **`edit.undo`/`edit.redo` while diverged keep a generic sentence.** They are
  gated by the same `InSync` test (`crates/ui-egui/src/app/commands.rs`) and are
  refused, which is correct, but the wording still says "reconnecting".

In the code, findings 9 and 10 of §5.26 remain (§4), each with its own slice and
test. Finding 8 is closed here.

### 5.34 Finding 10: a token rotation keeps the connection it belongs to (run 2026-11-11)

**The finding.** §5.26 #10, as CodeRabbit put it against `af9a43a2`: "a session
event for the current session replaces the client and drops its unconfirmed
edits" (`apps/wordcraft-web/src/web.rs:243-274`,
`crates/collab-web/src/lib.rs:120-128`). The trigger is ordinary: the cabinet
repeats `wordcraft:session` before the JWT expires, with the same `session_id`
and a fresh `token`.

**What was lost.** `HostState::on_event` answered `HostAction::Connect` for
every `Session`, so the shell built a second `Collab`, and `Collab::connect`
built a second `Client`. Everything the session had earned went with the old
object: `Client::pending` — the operations this tab planned and the room had not
confirmed, resent after a `welcome` under their original ids (ADR-0003, §5.5) —
the participant list, the version and the node map, the undo/redo stacks, and
the etag of the last commit. A writer could type, watch the text stay on screen
and lose it from the room's copy: the divergence class ADR WC-0006 exists about,
reached through a message nobody thought of as an ending. `Collab::renew_token`
already did the right thing and had no caller — the host path bypassed it by
replacing the object.

**The decision.** `docs/collab/adr/0007-token-renewal-keeps-the-connection.md`
(ADR WC-0007), with three options: ignore the repeated message (A), renew in
place (B, chosen), or copy `pending`/`participants` into a new object (C). A was
rejected because the transport reconnects on its own after any drop and the next
`auth` frame would then carry the **superseded** token — a rotation that arrives
before an expiry would turn into a session the server refuses. C duplicates
state ownership and cannot move a `Client`'s `seq` counter. B keeps the object:
same `Collab`, same `Client`, new credentials.

Five rules, as implemented:

1. Connection identity is `(session_id, ws_url)`. The token is what rotates, so
   it is not part of it; another URL is another room and opens a new connection,
   because the transport keeps the URL it dialled.
2. `HostAction::Renew` never builds a second `Collab`. `Collab::renew(&session)`
   refuses a session whose `session_id` differs (`false`; the shell dials) and
   refreshes the stored URL and token on the joined object otherwise.
3. An unchanged token is not a reason to reconnect: `renew` calls `renew_token`
   only when the token actually changed, so a duplicate host message costs
   neither a socket nor a `pending` blip in the cabinet.
4. A renewal with no live client dials. A first dial that failed, or a shell that
   only saw the renewal, has nothing to renew; the `Renew` arm falls back to the
   `Connect` body.
5. The role is the server's. `HostSession::role` is read when a connection opens;
   afterwards the room's `welcome` carries the role, so a repeated message cannot
   widen a viewer's rights locally.

**The change.** `crates/collab-web`: `HostAction::Renew(HostSession)`;
`HostState::on_event` compares the stored session's `session_id` and `ws_url`
and leaves `connected` alone on a renewal; `Collab::renew` (next to
`renew_token`). `apps/wordcraft-web/src/web.rs`: `apply_host_events` gained the
`Renew` arm and the old `Connect` body became `WebShell::connect`, so both arms
share one dial path. No wire change, no server change, no `craft-web-host`
change: `Message::Session` may simply be sent again, which `host-bridge.md` now
says out loud.

**A drive-by the gate forced.** The mandated clippy list (`-p wordcraft-web`
pulls `wordcraft-ui-egui` into the lint graph) failed on
`crates/ui-egui/src/collab.rs:279`: `needless_return` on the `return` that ends
the room-commit branch. Removing it would have changed wasm behaviour — the
host-commit path below it is `cfg(target_arch = "wasm32")`, so on the native gate
it looks unreachable and on the target it is not. The statement keeps its
`#[allow(clippy::needless_return)]` with that reason. The lint is not new: it
came in with §5.33's `3b05f844`, whose clippy run did not report it; whatever
the cause, the gate is green now, on the same command §5.28–§5.33 record.

**Tests.** `crates/collab-web/src/lib.rs`:

- `a_repeated_session_renews_the_same_connection_and_keeps_an_unconfirmed_edit`
  — the live session holds one unconfirmed operation and one peer; a repeated
  `Session` with a new token answers `Renew`, `renew` keeps the object, the new
  token is in it, the status is `Connecting` for the reconnect, `client_id` is
  unchanged, `pending` still holds the operation and the participant is still
  listed. `dials == 0`: the shell's `Renew` arm built no second `Collab`.
- `a_repeated_session_with_the_same_token_keeps_the_socket` — the same message
  with the same token leaves the status `InSync`.
- `a_session_for_another_id_still_replaces_the_connection` — `Connect`, and
  `Collab::renew` returns `false` for it: the old client does not absorb another
  session's message.
- `a_session_for_another_url_is_a_new_connection` — the URL half of rule 1.

Checked against the pre-fix behaviour on the build host: with
`same_connection` forced to `false` (the old answer), the two renewal tests fail
on `Connect` vs `Renew` and the two replacement tests still pass; the patch was
reverted afterwards and the tree is clean.

**What ran** (build host `agents`, on the tree above):

- `cargo fmt --all -- --check` EXIT=0;
- `cargo clippy -p wordcraft-collab -p wordcraft-collab-app
  -p wordcraft-collab-web -p wordcraft-web --all-targets -- -D warnings`
  EXIT=0;
- `cargo test` over the six crates: `wordcraft-collab` 61 passed,
  `wordcraft-collab-server` 66 passed and 1 ignored (the LibreOffice commit
  gate), `wordcraft-collab-client` 20, `wordcraft-collab-doc` 116,
  `wordcraft-collab-app` 42, `wordcraft-collab-web` 9 (4 new) — **314 passed,
  0 failed, 1 ignored** (310 before this slice);
- `cargo check -p wordcraft-web --target wasm32-unknown-unknown` EXIT=0 — the
  shell's new arm is wasm-only, so a native `--all-targets` run does not compile
  it. The check reports two warnings from `ui-egui` (`unused imports
  block_behind/forget_nudge`, `unused variable system_arabic`) that are not in
  this slice's files and do not fail the check.

**Local head.**

| | |
|---|---|
| head | `b963a8db` — `collab: a repeated session renews the connection it belongs to` |
| files | `crates/collab-web/src/lib.rs` +159/−2, `apps/wordcraft-web/src/web.rs` +31/−16, `crates/ui-egui/src/collab.rs` +4, `docs/collab/adr/0007-token-renewal-keeps-the-connection.md` +175 (four files, +369/−18), plus this page and `host-bridge.md` |
| pushed | yes, as part of `50c68e3a`: the fork branch and PR #5834 carry this commit (§5.35) |

**What is left.** The commit is delivered in `50c68e3a` (§5.35); a maintainer's
approval of the runs on that head is still outstanding. Three things this slice
names and does not take:

- **The sender is the cabinet's.** No test here proves that the real cabinet
  page sends the repeated `session`; the crate's `Message::Session` and the app
  side are ready for it (mail-frontend M87 owns the page). A manual two-client
  rotation through the trunk build is still on the same list as §5.1's
  "what remains for a person".
- **Finding 9 is untouched.** The connection is still app-wide while documents
  are per window, so a connection made for one document can carry another's
  edits (§4, §5.26). It needs per-document `Collab`s, and this fix does not
  pretend to be a step towards it — the renewal is keyed by the session the host
  sent, not by the document.
- **A rotation that is never delivered leaves the stale token.** If the host page
  is gone, the reconnect is refused and the panel shows `error`; there is no
  retry that could invent a fresh token. Named in ADR WC-0007 §Consequences 5.

### 5.35 The head is delivered; the runs and CodeRabbit on `50c68e3a` (run 2026-11-11)

This entry records the state of PR #5834 after the fifteen commits of
§5.28–§5.34 left the local branch, and the one thing a maintainer still owes
it. No code changed: this commit touches `docs/collab/STATUS.md` only.

**The head.** `git rev-parse HEAD`, `git ls-remote fork
refs/heads/feature/realtime-collaboration` and `gh pr view 5834 --json
headRefOid` all answer `50c68e3ad07cd4114004ea292b1d6eb4739a2231`; the PR's
`headRefName` is `feature/realtime-collaboration` on `copoka/wordcraft` and its
`baseRefOid` is `4b0d39959968a34e462cb53099e76fb108d0e96f`. The branch is 94
commits over that base and the diff is 69 files, +30473/−22
(`git diff --stat 4b0d3995..HEAD`). `git log --oneline af9a43a2..HEAD` has the
fifteen commits of §5.28–§5.34, in the code+documentation pairs those entries
name.

**The merge state.** `gh pr view 5834 --json mergeable,mergeStateStatus,
reviewDecision,isDraft` answers `MERGEABLE`, `BLOCKED`, `""` and `false`. The
branch merges cleanly; `BLOCKED` is the required review and the unapproved
workflows, not a conflict, and no reviewer has recorded a decision.

**CI on the delivered head.** From `gh api
"repos/storytold/wordcraft/actions/runs?head_sha=50c68e3a…"`, five runs, all
created 2026-11-11 03:27:31Z:

| Workflow | Status |
|---|---|
| `Printing` | `action_required` |
| `SonarCloud` | `action_required` |
| `Packaging lint` | `action_required` |
| `Nix` | `action_required` |
| `CI` | `action_required` |

Nothing executed, for the fork rule of §5.21. The evidence that the tree builds
is still the build-host run of §3 and the `printing.yml` steps of §5.25, not a
CI run on this head.

**CodeRabbit on `50c68e3a`.** Its check started at 03:27:51Z and is
`in_progress`; `gh pr view 5834 --json reviews` has no review for that head, the
newest being `5848804525` on `af9a43a2` (2026-11-10 20:25:18Z, §5.27). There is
therefore nothing to triage in this pass: no finding of the new review is known
yet, and this entry does not guess at one. The next session reads the completed
review once and records its findings here rather than polling the check.

**What is left.** One maintainer's approval of the five runs above and a review
decision. Until the review on this head completes there is no new CodeRabbit
finding to answer; the ten of §5.26 are otherwise settled — findings 4 and 7 in
`4ecab864`, the rest in §5.28–§5.34, except 9 (per-document connections), which
is still open and named in §4. This page itself is a local commit on top of
`50c68e3a`; the envoy role cannot push, so it is one fast-forward for the owner.



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
| Cabinet commit callback | `apps/collab-server/src/cabinet.rs` |
| Run book | `docs/collab/README.md` |
| Operator view, callback contract, rollout | `docs/collab/production.md` |
