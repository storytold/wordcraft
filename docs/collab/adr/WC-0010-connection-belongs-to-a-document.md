# ADR WC-0010: a connection belongs to one document, and a document switch closes it

- Date: 2026-11-16
- Status: accepted for implementation; the code and its test are in `08f41b43`
  (`STATUS.md` §5.54)
- Scope: WordCraft implementation, client side: `crates/collab-web`
  (`Collab::document_id`/`carries`, `HostState::on_event` taking the open
  document, `HostState::document_changed`, `HostAction::Refused`),
  `crates/collab-app` (the same two methods on the desktop session),
  `crates/ui-egui/src/collab.rs` (the guard that closes a connection which
  answers for another document) and `apps/wordcraft-web/src/{web,host_session}.rs`
  (the shell's routing). No wire change, no server change, no document-model
  change.
- Finding: `STATUS.md` §4 and §5.26 finding 9, and §5.34 "Finding 9 is
  untouched": the connection is app-wide while documents are per window, so a
  socket opened for one document can carry another's edits.
- Acceptance criterion affected: `TASK.md` §2 "changes converge without loss or
  duplicates" — an edit must reach the room of the document it was made in and
  no other — and §1, "the feature flag off means no network call".

## Context

A window (a browser tab or iframe, a desktop app window) holds one `WordApp` and
one `Option<Collab>`. The session the host or `collab.enable` opened became that
field, and nothing tied it to the document that was open when it was opened:

- the seed the server builds a room from is the open document (`Collab::connect`
  takes `&Document`), so the room and the document were the same at join time;
- `after_command` sent every replicable command through `app.collab`, and `tick`
  polled and published presence through it, without asking which document was
  active;
- a window can change its document without closing: File › Open into an
  untouched blank document, a dropped file, the host's `open` message, or a
  switch to another open document tab (`Session::document_id` changes, or
  another document's id becomes active).

So the first document's room received the second document's operations, with the
second document's positions and ids. The room's answer then wrote that
document's edits into the second document. The host bridge made it easier to
reach: the cabinet sends `session` once, after the first document, and a user
who then opens another file in the same tab keeps that connection.

## Decision

The connection carries the document it was opened with, and the window closes it
when another document becomes active.

1. `Collab` records `document_id` at `connect` and answers `carries(document)`.
2. `HostState::on_event` takes the document the tab shows. It remembers which
   document each `session_id` was opened for (bounded at 64 bindings) and
   answers `HostAction::Refused` for a message whose id belongs to another
   document, instead of binding it here.
3. `HostState::document_changed(document)` returns the session a switch must
   close; the shell drops the `Collab`, whose transport closes the socket.
   `Collab::renew` still applies only to the same id, URL and document.
4. The UI's guard (`close_if_another_document`) runs before `tick` polls and
   before `after_command` sends, on both the web and the desktop paths: a
   connection that answers for a document the window no longer shows is closed
   and the command stays local. `viewer_refusal` also ignores such a
   connection.
5. `session-state` and `presence` are published only for a connection that
   answers for the open document.
6. The host contract is unchanged in shape but gains an obligation
   (`host-bridge.md`): `session` is sent for the document the host wants shared,
   after the document is open — which includes after a switch.

The price, named: the closed connection's **unconfirmed** operations go with it,
and the local document keeps what was typed. Rejoining resyncs from the room,
which may not hold an operation that never left the tab. This is the same price
§5.28/§5.29 already charge for a forced resync, chosen over the alternative —
sending another document's edits into a room — which loses correctness rather
than delivery.

## Consequences

- A document switch costs the session: the host must send `session` again for
  the new document (the cabinet already sends it after an open, ADR WC-0004).
  The desktop reopens it with `collab.enable`, which already reconnects when no
  session is live.
- A late renewal for the old session is refused with a status line
  ("Not joining collaboration session …: it belongs to another document"),
  rather than silently joining the new document into the old room.
- A host that reuses one `session_id` for several documents now cannot: each
  document needs its own session id. The sixty-fifth session id a tab sees is
  forgotten and then refused, which is the safe side for a pathological host.
- Two documents in one tab never share an operation queue, a client id
  (`session_client_id` already keys on the document) or a participant list: the
  old connection, with all three, is dropped.

## Test evidence

- `crates/collab-web`: `two_documents_in_one_tab_neither_carry_each_others_edits_nor_share_the_queue`
  — document 7's connection plans an edit, document 8 becomes active and the
  session is reported for closing, a late renewal for it is `Refused` while the
  binding stays, document 8's session connects with an empty queue, and
  `carries` is true only for the document a connection was opened with.
- `apps/wordcraft-web`: `a_second_document_in_the_same_tab_closes_the_firsts_session`
  — the shell's own routing: the first session is returned for closing, the
  renewal is refused for the second document.
- The whole gate at `08f41b43`: `STATUS.md` §5.54 —
  `cargo fmt --check`, scoped `clippy -D warnings` and 1467 tests in eight
  collab/UI targets green.
