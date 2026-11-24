# The first adapter increment: the session seam and its five cases

`crates/collab-adapter` is where a WordCraft document meets a live session. It is the first
increment of the adapter path RFC-0020 sets out: the shared layer owns the contracts, each
application owns an adapter of its own pull request, and the app's document is edited only through
its own primitives.

## Why the seam is app-side for now

RFC-0020's landing order puts the live-session contract in a crate beside `craft-cloud`, with the
in-process mock and the degrade-to-absent tests, before any backend or app adapter. That crate does
not exist yet — the storage contract (`crates/cloud`, `crates/cloud-local`) and the presence
control (`crates/ui/src/presence.rs`) are there, the session, the invites, the conversation and the
`Texts` boundary are not. The gap is written up in storytold/craft-libs#3235.

So this crate names the seam itself, in the RFC's vocabulary, and nothing else:

- it carries **no wire type**: no envelope, no message kind, no encoding, no operation id space on
  the wire. The payload the seam passes is the application's own `Edit`, and a backend may put
  anything it likes on the socket;
- it does **not** pin an unmerged revision: the branch is cut from `origin/main` and the
  `craft-libs` revision under it is unchanged;
- when the shared trait lands, it replaces `seam::Session`, a thin `impl` delegates to the backend
  below it, and the document half (`ids`, `edit`, `session`) does not change. That is why the seam
  goes first.

## What the increment holds

| module | lines | what it does |
|---|---|---|
| `seam` | 167 | the trait: join at a role and a sequence point, ordered submission against the version applied, the operations after that version, presence per peer, the text a removal orphaned. `RefusalCode` says why a submission did not land. |
| `ids` | 190 | session block ids: existing blocks numbered alike on every replica; a peer's own ids minted in the space the relay grants it (RFC-0020 constraint 3); paths rebased by id after a structural edit; a gone id is refused, never re-pointed. |
| `edit` | 234 | four edits — insert text, delete a range, remove a paragraph, insert one — applied through `insert_text`, `delete_range`, `insert_block`, `remove_block`; each reports the inverse a per-user undo stack stores; every refusal is decided before the document changes. |
| `relay` | 221 | the in-process backend the cases run against: one sequence number that outlives a peer's handle, the journal a reconnect replays, one authoritative role per peer, presence latest-wins, and the stale rule (a text edit built behind another change of the same paragraph is refused and retried, never applied out of order). |
| `session` | 186 | a replica: joins, applies what the relay sequenced in that order, submits against the version it applied, replays only the tail on reconnect, rebuilds from the document only when the journal refuses the point, refuses a viewer before a submission leaves the process. |

The document model is `wordcraft_doc::Document`; nothing here re-encodes a part it does not model.
Bodies only, top-level blocks only: tables, parts, lists, formatting and their inverses stay with
the collaboration branch's `collab-doc` and move behind the same seam next, not in this change.

## The five cases (`tests/conformance.rs`)

| case | what it shows |
|---|---|
| two clients converge and a late joiner catches up | two replicas edit different paragraphs and the same one (the second submission is refused `Stale`, synced and retried); a third joins afterwards, replays the journal and agrees byte for byte with both and with the relay's copy |
| a reconnect replays the journal it missed | one peer works on while the other is away; the returning peer asks for the version it applied and receives only the tail; it does not resubmit, and the journal length is unchanged. A peer asking for a point the journal cannot serve is told `Resync` |
| a structural edit against a text edit on the same scope | the text is sequenced first, then the paragraph is removed; every replica drops it the same way, the removed text does not resurrect the paragraph, and the text the second edit carried is handed back to its author through `Recovered` |
| a viewer is refused | the adapter refuses before the submission leaves the process *and* the backend refuses a submission that bypasses it; nothing is sequenced, the document does not move. Presence is published by the viewer and read latest-wins by the editor's participant list |
| a hundred-page document round-trips through `.docx` | 3000 paragraphs paginate at 100+ pages identically on both replicas and on the relay; a custom XML part related from the main part, which the model does not read, survives the session *and* the save byte for byte; the saved file opens with the edited text and the same pagination |

## What this increment does not restore

Two gaps are named here rather than fixed, because both need per-character or per-fragment data
that this increment's payloads do not carry:

- **An inverse restores text only.** `Effect::inverse` of a removal re-creates the paragraph with
  default paragraph and character properties and without the non-text content the removal dropped;
  a deleted range's inverse re-inserts its characters without their formatting. The undo stack
  must not present these as a full restore until the fragment-carrying inverse (the branch's
  `collab-doc`) sits behind the same seam.
- **Accepted typing that another peer's removal takes away is not in the recovery list.** The list
  holds the text of edits the relay *refused* (`Gone`): an insertion under a removed anchor, a text
  edit whose paragraph is gone. When a paragraph that already holds somebody's accepted typing is
  removed, every replica drops that typing — one visible result — and no recovery entry is made,
  because saying whose characters were dropped needs per-character authorship, which arrives with
  `Texts`.

## What the next increment owes

1. The shared contract supersedes `seam::Session`; the adapter delegates instead of declaring.
2. The rest of the branch's document half (tables, parts, lists, formatting, undo) behind the seam.
3. Scope locks for commands that do not commute, and the `crdt.*` passthrough for text once `Texts`
   exists; this increment orders text through the relay and says so.
4. The cabinet commit surface (`commit` → `committed {version, etag}`, a stale etag refused, a
   `409` offering the room's bytes as a copy) and the lease, in the web/desktop wiring.
