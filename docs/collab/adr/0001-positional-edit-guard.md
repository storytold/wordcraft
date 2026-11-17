# ADR WC-0001: a positional edit that outlived its base is refused, not applied at a stale offset

- Date: 2026-11-11
- Status: accepted
- Scope: WordCraft implementation, `crates/collab` (`Room::submit`) and
  `crates/collab-client` (refused queue). Protocol-wide decisions stay in the
  collaboration bus (`ADR-0001`…`ADR-0021`); this record numbers separately and
  covers one implementation defect inside ADR-0004 ("optimistic apply, the
  server is the authority") and ADR-0003 (`baseVersion` is context, not a lock).
- Finding: `STATUS.md` §5.26 finding 1 — "concurrent positional edits are
  applied at a stale offset; the author skips its own echo, so a replica can
  diverge without a resync" (`crates/collab/src/room.rs`, CodeRabbit Major).
- Acceptance criterion affected: `TASK.md` §2 "Changes converge without loss or
  duplicates".

## Context

Two replicas edit one paragraph. Both have applied their own primitive
optimistically at the byte offsets their document had, send it with
`baseVersion` = their version, and **skip their own echo**: `SyncState::apply`
returns early for an operation whose `op_id.client` is this client, because the
edit is already in the document.

The room, however, applies every accepted operation to the *head* and computes
its delta there. When another operation landed between a client's
`baseVersion` and its own operation, that delta is addressed at server offsets
that describe a text without the client's pending edit. The client then applies
the foreign delta over its optimistic text, which shifts the wrong characters:
the pending edit moved every position after it, and the delta knows nothing
about that.

Example. Paragraph `abcdef`, Alice inserts `X` at offset 3 while Bob inserts
`Y` at offset 1, both at the same base; the room orders Alice first:

| | Alice's replica | Bob's replica | Room |
|---|---|---|---|
| Bob types locally | | `aYbcdef` | |
| Alice types locally | `abcXdef` | | |
| Alice's op lands | echo skipped | applies delta `insert Y@1` | `abcXdef` → `abcXdef` |
| Bob's op `insert Y@1` on head | delta `insert X@3` at 3 of `aYbcdef` → `aYXbcdef` | echo skipped, stays `aYbcdef`+… | `aYXbcdef` |

Bob's replica keeps a different document and nothing makes it resync: the
refusal path is never taken, the version advances and the peer looks healthy.
The same happens for a deletion at or before the pending offset — the delta is
computed on a text that already contains the other edit, so the client deletes
the wrong characters.

The defect is not "the offsets are wrong on the wire" — they are what the user
saw. The defect is that the client cannot apply a delta addressed at server
offsets on top of an optimistic edit the server has not seen, and the client is
never told that its assumption broke.

## Options

### A. Transform positional edits (OT table, or a client rebase)

The room transforms an incoming primitive against the operations accepted since
its `baseVersion`, or the client reverts its pending primitives, applies the
foreign delta and replays them at the original offsets. Both converge for text,
and the client keeps its text in the paragraph.

Cost: a table over `insert_text`/`delete_range`/`replace_range` × each source,
plus tie-breaking at equal offsets, plus the same question for `split_para`,
`merge_para`, cells, and commands whose payload carries a positional selection;
and, for the revert/replay form, a client-side inverse for every primitive
(the inverse exists inside the adapter, not in `collab-client`). Each of those
needs its own two-client tests, because a wrong transformation table fails
exactly like the current bug — silently. This is a design of its own, not a
slice that fits next to the finding.

### B. `baseVersion` guard with a forced resync (chosen)

The room refuses a positional primitive when an operation accepted after its
`baseVersion` changed the text of the same paragraph, with
`RejectCode::Stale` (retryable). The client already answers a `Stale` refusal
with a snapshot request (`refusal_status`, `Intent::Resync`), replaces the
document with the authoritative model (`SyncState::take_snapshot`) and
continues. The replica that was about to diverge adopts the server's state; no
replica can hold a document the server does not have.

The condition is deliberately narrow: same paragraph only. A stale edit in
another paragraph keeps landing (its offsets do not depend on the other
paragraph), so two people typing in different paragraphs never resync. Within
one paragraph, *any* concurrent change invalidates the offsets, including one
after the caret: the client applies the foreign delta at those offsets over its
own inserted text, which is off by the length it inserted (the example above
with `Y` at 5 would break the same way).

The guard covers the primitives whose payload is a byte offset:
`insert_text`, `delete_range`, `replace_range`, `split_para`, `merge_para`.
For a command the delta names only the command, so the room reads the stored
inverse instead: a `set_para_format` inverse that carries `text` is a command
that rewrote characters (`format.changeCase`, `format.enclose`, §5.10) and it
counts as a change of that paragraph too. Undo/redo requests are exempt: their
inverse is not applied optimistically at the author, so the author takes its
addresses from the room.

### C. CRDT

Rejected for the same reasons as ADR-0004: the document tree, layout and
`.docx` round-trip are not expressible as a CRDT without a second model, and
the criterion "no heavy CRDT stack" still holds.

## Decision

**B**, with the guard placed in `Room::submit` before the model runs, so a
refused operation never reaches the document, the journal or the undo stack.

Consequences, stated plainly:

1. **The convergence criterion is met in the sense that matters:** no replica
   can be left holding a document the server does not have. A stale positional
   edit becomes a `stale` refusal and a resync, which is visible in the
   status (`Resyncing`) instead of a silent difference.
2. **A concurrent edit inside one paragraph can cost the unconfirmed
   keystrokes.** The refusal is retryable, but the client resyncs instead of
   resending: the same offsets are exactly what is wrong, so resending them
   would recreate the bug. The refused operation is now dropped from the
   pending queue (`Client::on(Rejected)` for `stale` and `gone`), so a later
   reconnect cannot replay it either.
3. **`rfc.md` §4.4 is qualified.** Its row "two people type in one paragraph:
   both texts, the caret does not jump" is true while the operations are not
   concurrent in the room's order; when they are, the later one is refused and
   its author resyncs. The row now says so.
4. **Rebase remains the target.** When it lands it replaces the refusal for
   text primitives, keeps option A's tests, and this ADR is superseded. Until
   then the guard is the floor: a refusal that is tested, not a merge that is
   hoped for.

## Test evidence

- `crates/collab-doc/src/tests.rs`:
  `a_text_edit_on_a_paragraph_another_client_changed_since_its_base_is_refused`
  (refused `Stale`, retryable, nothing landed) and
  `a_stale_text_edit_in_another_paragraph_still_lands` (negative control: the
  guard is per paragraph, not per document).
- `crates/collab-client/src/tests.rs`:
  `a_refused_stale_edit_is_not_resent_after_the_resync` (queue dropped), while
  a `lease` refusal keeps its operation.
- `crates/collab-app/src/tests.rs`:
  `a_stale_positional_edit_resyncs_instead_of_diverging` — Alice's edit lands,
  Bob's optimistic edit at the same paragraph is refused, Bob takes the room's
  snapshot, and both replicas hold the same document JSON.
