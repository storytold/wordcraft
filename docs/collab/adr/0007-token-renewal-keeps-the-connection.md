# ADR WC-0007: a repeated session message renews the connection it belongs to

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, web side: `crates/collab-web` (`HostState`,
  `HostAction`, `Collab::renew`) and `apps/wordcraft-web/src/web.rs`
  (`apply_host_events`). Protocol-wide decisions stay in the collaboration bus
  (`ADR-0001`…`ADR-0021`); this record numbers separately. No wire change, no
  server change.
- Finding: `STATUS.md` §5.26 finding 10 — "a session event for the current
  session replaces the client and drops its unconfirmed edits"
  (`apps/wordcraft-web/src/web.rs:243-274` and
  `crates/collab-web/src/lib.rs:120-128` at `af9a43a2`; CodeRabbit Major).
- Acceptance criterion affected: `TASK.md` §2 "Changes converge without loss or
  duplicates", §4 "Participants see each other's carets within a second" and
  §6 (a session survives a token rotation).

## Context

The cabinet owns the collaboration session. It opens it with one
`wordcraft:session` host message (`Message::Session { session_id, ws_url,
token, role }`, `host-bridge.md`) and it **repeats that message before the JWT
expires**, with the same `session_id` and a fresh `token`. Nothing else in the
message changes.

`HostState::on_event` treated every `Session` as an opening: it dropped
`connected`, stored the session and answered `HostAction::Connect`. The shell's
`Connect` arm builds a **new** `Collab`, and `Collab::connect` builds a new
`Client`. Both halves of the session state therefore start from zero:

- `Client::pending` — the operations this tab planned and the room has not
  confirmed yet. They are resent after the next `welcome` with their original
  `opId`s (ADR-0003, and the dedup window survives a server restart, §5.5), so
  dropping them loses the writer's edit outright: the document on screen keeps
  it, the room never receives it, and the next commit builds the file from the
  room's version without it — the divergence class ADR WC-0006 was written
  about, reached by a route that ADR WC-0006 does not cover.
- `participants` — the presence list the cabinet shows in the participants
  panel and posts as `presence`. A new `Collab` starts with an empty list, so
  every peer disappears until the room's `welcome` arrives, and a peer's caret
  in the document is not drawn in the meantime.
- `state.version`, the node map, `undo`/`redo` stacks, the last `commit` and its
  `etag` — all of them describe the room this tab is joined to, not the tab.

The token, by contrast, is exactly the field the message exists to update. The
web session already had `renew_token` (`crates/collab-web/src/lib.rs`, mirroring
`crates/collab-app/src/lib.rs:642` and `crates/collab-ws/src/lib.rs:116`): it
stores the token, sets `Connecting` and asks the browser transport to drop the
socket and dial again. Its next `auth` frame carries the new token; the
unconfirmed operations, the version and the participant list stay where they
are. No caller used it — the host path bypassed it by replacing the object.

There is a second, quieter half of the bug. A repeated message whose token is
**identical** (`set_token` is idempotent; a host may resend the same session
after a reload of its own state) also tore the connection down, so a renewal
that changed nothing forced a resync-shaped blip and a `Connecting` status the
cabinet reports as `pending`.

## Options considered

**A. Ignore a repeated `session` for the same `session_id`.** The live socket is
already authenticated; the token is only read in the `auth` frame. This is the
smallest change and it cannot drop anything. Rejected: it keeps a token the
cabinet has told us is superseded. The transport reconnects on its own after
any drop (250 ms doubling to 10 s, `socket.rs`), and the next `auth` frame would
then carry the **old** token — refused by the server, with the tab stuck in
`Offline` until the host sends a message it has already sent. The cabinet pushes
the new token because the old one expires; ignoring it turns "expires at the
wrong moment" into a lost session.

**B. Renew in place: same connection, new credentials (chosen).** A repeated
`session` for a `session_id` this shell has already dialled updates the stored
session and hands the new token to the **existing** `Collab`; the object, its
`Client`, its unconfirmed operations and its participant list stay. A message
that also changes `ws_url` is not the same connection (the transport keeps the
URL it dialled and `reconnect` re-dials that one), so it opens a new one. A
message with another `session_id` is another session and replaces the client,
as before.

**C. Keep the client and re-dial with a fresh object** — build a new `Collab`
but carry `pending` and `participants` over. Rejected: it duplicates state
ownership across two objects, needs a way to extract pending operations from a
`Client` (they are keyed by a `seq` counter that would have to move too), and
solves nothing A and B do not.

## Decision

Five rules:

1. **Connection identity is `(session_id, ws_url)`.** `HostState::on_event`
   answers `HostAction::Renew(session)` when the stored session has the same
   `session_id` **and** the same `ws_url`; otherwise it answers `Connect`, as
   before, and drops `connected`. The identity is deliberately not the token:
   the token is what rotates.
2. **A renewal never builds a second `Collab`.** `Collab::renew(&HostSession)`
   is the only thing the `Renew` arm calls. It refuses a session whose
   `session_id` differs (`false`, the caller installs a new connection), and
   otherwise refreshes the stored URL and token on the object that is already
   joined.
3. **The same token is not a reason to reconnect.** `renew` calls
   `renew_token` only when the token actually changed. An identical repeat
   refreshes the stored session and leaves the socket, the status and the
   document untouched — the cabinet's `session-state` does not blink to
   `pending`.
4. **A renewal without a live client dials.** The shell's `Renew` arm falls
   back to `Connect` when it holds no `Collab`, or one for another session: a
   first dial that failed, or a shell that only saw the renewal. `HostState`
   does not know whether the dial succeeded, so it must not decide this.
5. **The role is the server's.** `HostSession::role` is read once, when a
   connection is opened; after that the room's `welcome` carries the role the
   server assigned and `Collab` takes it from there. A renewal that repeats or
   changes `role` neither promotes nor demotes the tab locally, so a stale host
   message cannot widen a viewer's rights.

## Consequences

1. **A token rotation no longer costs an edit.** The `Client` keeps
   `pending`; after the reconnect's `welcome`, `on_text` resends every
   unconfirmed operation under its original `opId` (the path ADR-0003 and §5.5
   already use), and the server deduplicates any it had applied. The document on
   screen and the room therefore converge on the same operations.
2. **Presence survives the rotation.** The participant list, the peer carets
   and the panel keep what they had while `Connecting`, and the first
   `publish_presence_at` after `InSync` republishes this tab's caret. Peers see
   one reconnect, not a leave/join pair, because the `client_id` is unchanged.
3. **A real token change still costs a round trip, and it is visible.** For the
   length of the reconnect the status is `Connecting`, so `can_write` and
   `can_commit` are false and the edit gate refuses a keystroke with the usual
   wording; the cabinet's `session-state` says `pending`. That is the price of
   using the new credentials, and it is the same price a dropped socket already
   charges. An identical repeat (rule 3) has no price at all.
4. **`renew_token` is no longer dead surface.** The web path uses it, so the
   desktop and the browser rotate a token the same way when the desktop gets a
   host bridge.
5. **Not covered: a rotation that arrives while the tab is offline.** The
   transport reconnects with the token in the object; if the host sent the new
   token before the socket dropped, the rejoin uses it, and if it sends it after,
   this path applies it. A rotation that is never delivered (the host page is
   gone) leaves the stale token, and the server refuses the rejoin — out of this
   fix's reach, and the panel shows `error` rather than pretending.
6. **Not covered: two `session` messages in one frame batch with different
   tokens.** They are applied in order; the first reconnects, the second updates
   the token again and reconnects again. Both keep the client, so nothing is
   lost; the redundant dial is host behaviour no client can deduplicate.
7. **`ws_url` changes are new connections.** A token renewal that also moves the
   server is a migration, not a renewal: it opens a new `Collab`, with the
   consequences of the old bug — because the old socket points at a different
   room and its unconfirmed operations cannot be replayed into the new one.

## Test evidence

`crates/collab-web/src/lib.rs` (`mod tests`):

- `a_repeated_session_renews_the_same_connection_and_keeps_an_unconfirmed_edit`
  — a session `s1` is opened and installed; the writer's operation is in
  `Client::pending` and a peer is in `participants`; `on_event(Session("s1",
  new token))` answers `Renew` (not `Connect`), `Collab::renew` keeps the same
  object, the token is the new one, `pending` still holds the operation,
  `client_id` and the participant list are unchanged, and the status is
  `Connecting` for the reconnect.
- `a_repeated_session_with_the_same_token_keeps_the_socket_open` — the same
  session id with an unchanged token: `Renew` again, and the status stays
  `InSync` (no reconnect), so a duplicate host message is free.
- `a_session_for_another_id_still_replaces_the_connection` — another
  `session_id` answers `Connect` (the shell installs a new client), and
  `Collab::renew` with that session returns `false` instead of silently
  applying it.
- The same tests cover rule 1's URL half:
  `a_session_for_another_url_is_a_new_connection`.

`apps/wordcraft-web/src/web.rs` holds the three-line `Renew` arm: it stores the
session, calls `Collab::renew`, and falls back to the `Connect` body when no
live client matches. The wasm-only module is checked with `cargo check -p
wordcraft-web --target wasm32-unknown-unknown` (the gates below), because a
native `--all-targets` clippy run does not compile it.
