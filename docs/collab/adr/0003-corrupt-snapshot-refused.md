# ADR WC-0003: a snapshot that cannot be opened refuses the join, it is not an empty room

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, `apps/collab-server` (`Store::load`, the room
  table and `/metrics`). Protocol-wide decisions stay in the collaboration bus
  (`ADR-0001`…`ADR-0021`); this record numbers separately and covers one
  implementation defect inside ADR-0005 (the server snapshots the room) and
  ADR-0011 (the snapshot is the model plus the source `.docx`).
- Finding: `STATUS.md` §5.26 finding 2 — "a snapshot that cannot be read or
  parsed becomes an empty room, which can then be persisted over the stored
  file" (`apps/collab-server/src/lib.rs:240-270`, CodeRabbit Major).
- Acceptance criterion affected: `TASK.md` §3 "nothing the server cannot model
  is lost" and §2's "without loss": losing a stored document to a blank room is
  the largest loss the server can cause.

## Context

A room is created on the first join for a document. `AppState::room_for` called
`Store::load`, which returned `Option<Snapshot>`: `None` covered both "no
snapshot was ever written" and every way a stored snapshot can fail — an
unreadable `model.json`, a `meta` that is missing or has a `version` that is not
a number, a model that does not parse as a `Document`, a model over the size
bound. All of them fell into the same `unwrap_or_else`, which built a room from
`Document::new()`.

The blank room is not harmless. It is a normal room: the first operation makes it
dirty, `persist` writes it after 500 operations or on `leave`, and
`persist_all` writes every open room on SIGTERM. `save` rotates the previous
pair to `prev/`, so the damaged file survives exactly one rotation — after the
second snapshot it is gone, and the document is an empty page the participants
never asked for.

The two states are not equivalent and must not be conflated:

- **absent** — the document has never been opened on this server. A new room is
  correct, and the cabinet still holds the file (the host path, ADR-0009's
  commit callback).
- **corrupt** — a document was here and its only copy is damaged. Anything that
  writes a blank room over it destroys the copy silently.

`Option` carried no room for that distinction, so the caller could not make it.

## Options

### A. Keep the fallback, rotate the damaged pair further

Rejected: rotation is a one-step rollback, not a backup. The second snapshot
after the fallback (`prev/` is overwritten by `save`) leaves nothing to restore,
and the failure is still silent — an operator learns about it from a user who
lost a document.

### B. Distinguish absent from corrupt in the store, refuse the join (chosen)

`Store::load` returns `Result<Option<Snapshot>, Corrupt>`:

- `Ok(None)` — `model.json` does not exist: never snapshotted, a new room is
  correct;
- `Ok(Some(snapshot))` — every file read and every known `meta` line parsed, the
  model parses as a `wordcraft_doc::Document` and is within the model bound;
- `Err(Corrupt { file, reason })` — a snapshot is there and cannot be opened.

`room_for` turns `Err` into a refusal instead of a room: no entry is inserted in
the room table, so there is nothing to persist, nothing to rotate and nothing to
commit. The join is answered `Rejected { code: invalid, retryable: false }` with
a message that names the situation and not the server's paths, and the detail
(`file: reason`, never document content) goes to the log. The refusal is counted
in `wordcraft_collab_corrupt_snapshots_total`.

`retryable: false` is the honest answer: a retry opens the same file. The client
shows it as a failed state (`refusal_status`), not as a resync.

### C. Quarantine the file and start a blank room

Rejected: it turns silent loss into logged loss. The document still becomes an
empty page, the participants still see blank, and the next commit still replaces
the cabinet's copy with it. A quarantine is only useful once a person has
decided the file is unrecoverable; at that point removing the directory is the
same action with no extra state to grow.

### D. Refuse to start the server when any snapshot is damaged

Rejected: one damaged document would take every other document offline. The
refusal belongs to the document, not to the process.

## Decision

**B.**

The store owns the distinction because it is the only place that can see it: it
knows which file is missing and which one is unreadable. `save` keeps writing
the metadata last, so a model without metadata is an interrupted write and is
corrupt, not absent — the model was renamed into place before the pair was
committed.

The `meta` reader became strict for the keys it knows:

- `version` is required (the head the room restores);
- `epoch` may be absent (a snapshot written before the epoch was stored) and
  starts at 0, as it did before;
- a known key whose value is not a number is corrupt, not a default — starting at
  version 0 would move the head backwards and let a client replay journal
  offsets that no longer exist;
- `cabinet_version` and `cabinet_etag` come as a pair. Half of one is a torn
  write, and dropping the etag with it would drop the conflict check that keeps
  the next commit from overwriting an external edit.

Two files stay forgiving on purpose, because they are not the document:
`dedup.json` (the `opId` window) and `source.docx` (the markup kept for
unmodified paragraphs, ADR-0011). A damaged window costs a duplicated answer
across a restart; a missing source costs original markup on unchanged
paragraphs. Neither is text, and neither justifies refusing the document.

The window is nevertheless read only when it belongs to the snapshot beside it,
which it proves by carrying that snapshot's version and the SHA-256 of its
`model.json` (review 5855379673). A newer window left behind by a divergent
history — a restore that put `model.json` and `meta` back from one backup while
`dedup.json` stayed from another — can hold an answer for an edit the restored
model never had, and acknowledging that retry would silently drop the edit. A
window that cannot prove its snapshot is dropped, which costs a duplicate answer
instead.

## Consequences

1. **A damaged document is not silently replaced.** The join is refused, the
   file is untouched, and the operator has a path to act on: restore the
   document's directory from **one** backup — `model.json` **and** `meta` from
   the same snapshot, since `Store::load` takes the head version from `meta` and
   an older model beside a newer `meta` advertises a head the document does not
   have. `source.docx` and `dedup.json` belong to that same snapshot as well: a
   missing source costs original markup on unchanged paragraphs, and the window
   is used only when its stamp — the snapshot version and the digest of
   `model.json` — matches, so a newer window from a divergent history is dropped
   rather than answering a retry for an edit the restored model does not
   contain (review 5855379673). Dropping it can let such a retry land twice; the
   other way round, an acknowledgement would claim an edit the document never
   got.
   `corrupt_snapshots_total` and the log line name the document.

   **Removing the directory does not start a blank room.** It leaves the
   document unstored, and the next join has to seed the room from a client's
   copy: a session that may write, with the document open, offers it on connect
   (ADR WC-0004). A join without such a seed — a read-only session, or a client
   that sends no copy — is refused, so deleting files cannot create an empty
   page that would then be persisted over the document a client still holds.
   Removing the directory is therefore only safe when the operator knows which
   client will reopen the document first.
2. **A document the server has never seen still opens.** `Ok(None)` is the
   ordinary first-open path, unchanged, and the test that a fresh document gets
   a room at version 0 still passes.
3. **A client that meets a damaged document reconnects in a loop.** The
   transport reconnects on any closed socket with a 250 ms backoff, and a join
   refusal closes the socket. The loop is bounded by the backoff and the app
   shows `Failed` with the refusal message, but nothing tells the transport that
   a non-retryable refusal before `Welcome` is final. That is a client concern of
   its own: a join refusal that is not retryable should stop the reconnect
   attempt and let the UI offer an explicit retry. It applies to the full-room
   and lease refusals too, and is not fixed here.
4. **The size check is a refusal too.** A model over `store::MAX_MODEL` used to
   be logged and ignored, with the same blank-room result; it now refuses. The
   bound only moves which damaged files are visible, not what happens to them.
5. **No protocol change.** `Rejected` already carries a code and a
   `retryable` flag; the refusal reuses `invalid`. No new frame, no new field.

## Test evidence

- `apps/collab-server/src/store.rs`:
  `a_damaged_model_is_corrupt_not_absent` — a truncated `model.json` is `Err`
  on `model.json`, not `Ok(None)`;
  `a_snapshot_without_its_metadata_is_corrupt` — a model with no `meta` (the
  interrupted-write shape) is `Err` on `meta`;
  `metadata_that_is_not_a_number_is_corrupt` — `epoch=now` and `version=five`
  are damage, not 0;
  `half_a_cabinet_marker_is_corrupt` — a `cabinet_version` without its etag is
  damage;
  `a_missing_snapshot_is_none` — the absent case is still `Ok(None)`, asserted
  against the corrupt cases above;
  `ids_the_old_name_could_not_tell_apart_are_stored_apart` — `a/b` and `a_b`,
  and two ids that agree in their first 64 characters, get different
  directories, and one is never handed the other's snapshot;
  `an_old_snapshot_directory_is_adopted_once` — a snapshot written under the old
  name is moved to the name the digest gives it, and the id that never owned it
  is left with nothing rather than another document;
  `a_snapshot_of_another_id_is_not_adopted` — a directory whose `meta` names a
  different `doc_sha256` is left untouched;
  `a_window_without_the_snapshot_stamp_is_not_used` and
  `a_window_stamped_for_another_model_is_not_used` — a window that cannot prove
  which snapshot it belongs to is dropped instead of answering a retry for an
  edit the restored model does not have (review 5855379673).
- `apps/collab-server/src/tests.rs`:
  `a_corrupt_snapshot_refuses_the_join_and_is_not_overwritten` — with a
  truncated `model.json` and a `meta` on disk, a real client's join is answered
  `Rejected { code: invalid, retryable: false }`, `room_count()` stays 0,
  `corrupt_snapshots_total` is 1, and `persist_all()` (the SIGTERM path) leaves
  `model.json` and `meta` byte-for-byte unchanged. The same state still opens a
  room for a document with nothing stored, at version 0.
- `a_room_is_restored_from_disk`, `a_retry_after_a_snapshot_restore_is_not_applied_twice`,
  `a_shutdown_snapshot_keeps_every_open_room` and
  `a_restored_room_takes_kept_markup_from_the_source_docx` still pass: the
  ordinary restore path is unchanged.
- Checked against the pre-fix tree on the build host: with `room_for` falling
  back to an empty room, the join test fails at the first `match` — the client
  gets `Welcome`, not a refusal — and, after `persist_all()`, `model.json` holds
  the empty document instead of the truncated bytes, which is the finding
  reproduced.
