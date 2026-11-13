# ADR WC-0006: an edit that did not reach the room pauses the session

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, client side: `crates/collab-app`
  (`Status`, `Pause`, `plan_command`, `submit_plans`, `Collab::after_command`),
  `crates/collab-web` (the same path over the browser socket) and
  `crates/ui-egui` (the panel, the commit button and the edit gate). Protocol-wide
  decisions stay in the collaboration bus (`ADR-0001`…`ADR-0021`); this record
  numbers separately. No wire change, no server change.
- Finding: `STATUS.md` §5.26 finding 8 — "an edit whose planning or submission
  fails stays local while the session still reports `InSync`; a commit can omit
  it" (`crates/ui-egui/src/collab.rs:29-47` at `af9a43a2`; CodeRabbit Major).
- Acceptance criterion affected: `TASK.md` §2 "Changes converge without loss or
  duplicates" and "Commit creates a new version, original kept".

## Context

A local command travels in three steps. `before_command` clones the document
when a room is live and the command mutates the document; the engine then runs
the command **in this window**; `after_command` plans the difference and submits
the operations. The document at step 2 is already the new one — the room has not
seen anything yet.

Both remaining steps can fail:

1. **Planning.** `SyncState::plan` returns `Err` when the diff cannot address
   this document (`DiffError::MissingId` — a paragraph the server issued no id
   for) or when the command fallback has no addressable selection
   (`"{command} needs a session id for its selection"`).
2. **Submission.** `Handle::submit` returns `Err("the session task is gone")`
   when the transport task has ended; the browser's `send` returns `Err` when
   the socket is not open. With several planned operations, the failure can also
   land after the first ones were accepted, so the room holds part of the edit.

The old `after_command` propagated the error string to the status bar and
changed nothing else. `Status` stayed `InSync`, so `can_write` and `can_commit`
stayed true, the panel said "synchronized", and the next commit asked the room
to build the file **at the room's version** — which does not contain the edit.
`TASK.md` §2's convergence criterion was therefore reported as met while a
writer's paragraphs were silently left out of the stored `.docx`.

## Decision

Reconcile and pause, in five rules:

1. **A failed command leaves `InSync`.** `Status::Diverged` is a status of its
   own, not `Resyncing`: it says "this document holds an edit the room does not
   have", which is a different fact from "a handshake is in progress". Both
   gates (`can_write`, `can_commit`) already require `InSync`, so writing and
   committing stop without a second flag.
2. **Both failures are one outcome.** `plan_command` and `submit_plans` (in
   `wordcraft-collab-app`) map either failure to the same `Pause`: the status
   and one message that names the cause and says the commit is blocked. The
   desktop and the web session use the same two helpers, so the two transports
   cannot drift apart here.
3. **The session asks for the room's state, and only a snapshot ends the
   pause.** A pause requests a snapshot at once (desktop: `Handle::resync`;
   web: `ClientMsg::Resync`). The snapshot replaces this document with the
   room's confirmed one, so the divergence ends in a known state instead of a
   merge nobody defined. A `welcome` does **not** end it: a reconnect hands back
   the handshake but not the document, and the edit the room never received is
   still in this window. While the status is `Diverged`, a welcome asks for the
   snapshot again and the session stays paused.
4. **A transport that cannot even ask is not left paused.** If the resync
   request itself fails, the desktop reports `Failed("…")` (the task is gone;
   nothing will repair the session) and the web reports `Offline` (the socket is
   not open; the transport reconnects and rejoins). Neither hides the reason:
   `last_error` keeps the message.
5. **The commit is refused with the reason.** The owner keeps a disabled Commit
   button whose hover text says why, `request_commit` reports the reason when it
   is called from a menu or a shortcut, and the edit gate refuses the next
   document command with the same wording. A read-only role is no longer able
   to reach the host-owned commit path through this branch either.

The pause is not a new protocol frame, a new server rule or a new snapshot
format. It is the client admitting a state the server already models: the
document is the replica's, the room's version is authoritative.

## Consequences

1. **A commit can no longer omit an edit the writer can see.** The worst case
   after a pause is a refusal the writer can read, not a stored file missing
   their paragraphs.
2. **The price is the one ADR WC-0002 already names.** The resync continues from
   the room's snapshot, so the edit that never reached the room — and, in the
   partial-submission case, nothing that did reach it — is dropped locally. The
   alternative (keep editing, retry later) needs the offline queue that does not
   exist; `STATUS.md` §4 has said so since the first slice.
3. **The two hosts say the same thing in their own words.** The panel reads "out
   of sync — an edit did not reach the room"; the cabinet's `session-state`
   reports `pending`, because a snapshot is on its way and the contract has no
   word for this state. The cabinet gets no detail by design: `session-state`
   carries a status and participants, and the reason stays in the panel.
4. **Editing pauses before the damage can stack.** `viewer_refusal` runs before
   the engine, so while the status is not `InSync` no second local edit is
   produced on top of the first; when the status returns to `InSync` the
   document is the room's.
5. **Not covered: a plan that returns nothing for a changed document.** If the
   diff ever answered "no changes" for a command that changed the document, the
   session would stay `InSync`; that would be a diff defect, and the guard would
   have to compare documents per keystroke. Named here so the hole is visible
   rather than implied.
6. **Not covered: `edit.undo`/`edit.redo` while diverged.** They are gated by the
   same `InSync` test and are refused with a generic "reconnecting" sentence
   (`app/commands.rs`). The refusal is correct; the wording is left for a later
   pass because undo has its own vocabulary for a refused step (ADR-0010).
7. **A reconnect no longer talks the session back into `InSync`.** The welcome
   path used to set `InSync` unconditionally, which by itself could hide a
   divergence across a dropped socket; it now keeps `Diverged` and asks for the
   snapshot again. The web session has the same rule, so a rejoin cannot make
   the panel say "synced" while the local document still holds the edit.
## Test evidence

`crates/collab-app/src/tests.rs`:

- `a_command_that_cannot_be_planned_pauses_the_session_before_a_commit` — a
  session whose map has no id for the paragraph an insert landed in. `plan_command`
  answers `Pause { status: Diverged, … }`, the message names the cause and says
  the commit is blocked.
- `a_command_that_cannot_be_submitted_pauses_the_session_before_a_commit` — the
  plan is real (one `replace_range` against a room that issued the ids) and the
  submit closure refuses it as a dead transport does; the outcome is the same
  `Pause`, and `room.version()` is still `0`, which is exactly the version a
  commit would have built.
- `a_command_that_reaches_the_room_does_not_pause` — the same plan with an
  accepting transport stays `Ok`, so the guard is not a blanket refusal.

Checked on the build host that the first two fail without the fix: with the
`Pause`'s status set back to `Status::InSync` (what the old `after_command`
left in place), both fail on their first status assertion while the plan and the
submission still return `Err`. The patch was reverted afterwards; the tree is
clean. (The helpers are new surface, so the pre-fix tree cannot run them at all;
reverting the status is the honest analogue of "the test fails without the
fix".)
