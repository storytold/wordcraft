# Live sessions (co-editing)

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-16 · **Change:** minor (craft-live mode: the web build edits Nextcloud files live; previously minor: document chat, protocol 2.1; previously major: created: plugin protocol 2 in WordCraft, schema 1) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

Several people edit one document at the same time, each in their own WordCraft, and see each
other's carets, selections and edits as they happen. This is WordCraft's equivalent of Word's
co-authoring. It's **native**: WordCraft merges the edits itself (a CRDT, Yjs through the
[`yrs`](https://crates.io/crates/yrs) crate), so two people typing in one paragraph merge
character by character; it shows who's there and keeps a per-person undo. Moving the bytes
between people is a **transport add-in**'s job (MQTT, a local relay, a y-websocket server,
Nextcloud…), which never needs to understand a document. With a room key the content is
encrypted end to end, so the transport only ever sees ciphertext.

The protocol is shared with GridCraft and DeckCraft:
[craft plugin protocol 2](https://github.com/sirmmo/craft-collab/blob/main/docs/protocol-2.md)
(push channel, rooms, `collab/*` messages, encryption, limits). This page is WordCraft's part: the
shared document (**schema 1**, normative) and how the app behaves. craft-collab's
[`docs/schemas/wordcraft-1.md`](https://github.com/sirmmo/craft-collab/blob/main/docs/schemas/wordcraft-1.md)
is the summary for other implementations, with the transcripts that check an app against it;
the two agree, and this page has the detail that one leaves to WordCraft.

## Using it

**Share**, at the right end of the tab strip. Not in a session, it opens the Share dialog: start
a live session from this document or join one, pick the connection (the transport add-in), your
name and colour, and for a new session whether it's encrypted end to end; joining takes the
invitation link. The dialog runs the transport's own `share` or `join` command; its answer names
the room, and WordCraft joins it. With no transport add-in loaded, Share does what it does
without live sessions: File › Export, a copy to send. Joining replaces the open document with the room's,
so a document with unsaved changes asks first ("Discard my unsaved changes").

In a session, the button carries a dot for the session's state (green: live; amber: connecting or
joining; red: offline, editing goes on here) and opens the session popover: the state and room,
whether it's encrypted, the transport's message, the people here (circle, name, "(you)"; click
one to go to their caret), **Copy Invitation** when the transport gave a link, and **Leave Live
Session** (the document stays open with every edit, as an ordinary document; not saved
automatically).

At the right end of the title bar, the people in the session are overlapping circles with their initials on
their colour, the others first and then you, at most four and then "+N" (hovering lists the rest).
Hover a circle for the name; click it to select what that person has selected (or put the caret
where theirs is).

On the page, others' carets are bars in their colours with a name tag, their selections are
shaded in their colours, and paragraphs they change carry a fading amber mark in the margin for
two seconds. The status bar shows "Live session", how many others are there, the state, and the
room's problems ("Wrong room key?", "This room is for gridcraft 1").

Undo and Redo, while in a session, undo **your** changes only; typing that goes on is one step,
as outside a session. Others' edits never enter your undo history. Saving works as usual: each
person saves their own copy.

## Commands

| Command | Params | Does |
|---|---|---|
| `collab.share` | `{plugin, room, user?, role?, key?, title?, invitation?}` | Starts a session from the open document (no params: the Share dialog) |
| `collab.join` | `{plugin, room, mode: share\|join, user?: {id, name, color}, role?: edit\|view, key?, path?, title?, invitation?}` | Joins or starts a session (no params: the dialog). `share` seeds the room from the open document (or `path`); `join` replaces the document with the room's (refused while the open one has unsaved changes), or merges into `path` |
| `collab.leave` | `{room?}` | Leaves |
| `collab.status` | `{}` | `{rooms: [{room, plugin, state, peers, listed, role, encrypted, clientId, user, title, invitation, message, canUndo, canRedo}], transports: [{id, transport, label}], user}` |
| `collab.setUser` | `{name?, color?}` | Your name (the author name, File › Options › General) and colour (`#RRGGBB`) |

`plugins.run` returns a transport's `collab.join` in its result (`collab: {join}`, with the
invitation from `result.link` added); the interface joins it, a script passes it to
`collab.join` with `plugin`.

## Schema 1 (normative)

The Yjs document a WordCraft room shares. `collab/joined` says `app: "wordcraft"`, `schema: 1`.
Text indexes count UTF-16 code units, as Yjs does everywhere; an inline object (an embed) counts
one.

### Roots

| Root | Yjs type | Holds |
|---|---|---|
| `meta` | Map | `app` = `"wordcraft"`, `schema` = `1`, `title` (the file name when shared, else empty) |
| `body` | XmlFragment | the main story: `<p>` and `<table>` elements (below) |
| `headers`, `footers`, `notes`, `textBoxes` | Map | story id → story map: headers; footers; footnotes and endnotes; the text of text boxes and shapes |
| `comments` | Map | comment id → comment map (fields and text) |
| `styles` | Map | style id → the style (WordCraft's `Style` as JSON: `name`, `kind`, `basedOn`, `next`, `para`, `chr`…); `@defaults` → `{defaultChr, defaultPara, defaultParaPresent, latentStyles}` |
| `styleOrder` | Array | style ids, in the style sheet's order |
| `numbering` | Map | `a<id>` → abstract list definition (`AbstractNum`), `n<id>` → list (`Num`: `abstractId`, `startOverrides`, `levelOverrides`) |
| `settings` | Map | `settings`, `lastSection`, `core`, `sources`, `customProps`, `revisions`, `footnoteSeparators`, `endnoteSeparators`, `ribbon`: each WordCraft's JSON of that document property, whole |
| `media` | Map | media key → bytes (pictures, embedded objects) |
| `files` | Map | package parts kept verbatim (custom XML, `vbaProject.bin`…), part name → bytes |

JSON values are stored as Yjs values (maps, arrays, strings, numbers, booleans, null). Numbers
are JavaScript numbers; WordCraft writes integral ones as Yjs integers and reads them back as
JSON integers, so the same value is always the same Yjs value. Keys WordCraft doesn't know are
ignored, and a property that doesn't read (another version's) is dropped rather than the rest.
A room without `meta` yet is empty; `meta` naming another app or schema refuses the room.

### Blocks

**Paragraph** `<p>`: attributes `pPr.<key>` (one per key of WordCraft's paragraph properties as
JSON: `pPr.style` the style id, `pPr.align`, `pPr.indentLeft`, `pPr.numId`, `pPr.level`…),
`rPr` (the paragraph mark's character properties, an object, when not empty), `sectPr` (the
section's properties, when the paragraph ends a section). One XmlText child: the text.
Characters are as typed (`\t`, `\n` line break, U+000C page break, U+000E column break…).
Formatting is Yjs text attributes, one per key of WordCraft's character properties as JSON
(`bold`, `italic`, `size`, `font`, `color`, `style`, `lang`, `highlight`, `underline`,
`boldCs`…); a missing key is not set. Inline objects are embeds: the object's JSON (tagged by
`type`: `image`, `field`, `noteRef`, `bookmarkStart`, `commentStart`, `equation`, `shape`…),
with formatting attributes like a character. U+FFFC never appears in the text.

**Table** `<table>`: `tblPr` (table properties), `grid` (column widths, points), `controls`
(content controls, when any). Children `<tr>` (`trPr`, `controls`), each with `<tc>` children
(`tcPr`: span, vertical merge, shading…; `controls`) holding blocks. A cell reads as ending with a
paragraph; a grid narrower than the rows gets more columns. Tables nest 16 deep.

Paragraphs, tables, rows and cells are identified by their Yjs elements (their item ids); there's
no id attribute.

### Stories, comments, ids

A story map: `kind` (`header`, `footer`, `footnote`, `endnote`, `textBox`), `content`
(XmlFragment of blocks), `body` (text boxes, when not the default: `vert`, `anchor`, `next`,
`inset`, `autoFit`). A comment map: `story` (its text's story id), `content`, `author`,
`initials`, `date`, `parent` (replies), `resolved`, `durableId`, `dateUtc`, each its own key.

Story (part), comment and list ids are integers below 2³². Ids below 65,536 are the document's
when it was shared. In a session WordCraft makes new ones in its own block of 65,536 ids from
65,536 × (1 + clientId mod 32,767), never giving one twice (an id an undo gave up isn't reused),
so two people adding a footnote, a comment or a list at once never collide. Stories and
comments are read together, so updates arriving out of order can't hide one behind another's id.

### Edits

Each committed command is one Yjs transaction and one `collab/update`: WordCraft compares the
document before and after the command and writes the difference.

- **Blocks** of a story pair up by identity, then by likeness (the common start of the text
  counts most), so Enter keeps the paragraph with the text before the caret and inserts a new
  `<p>` after it with the rest; Enter at the start inserts an empty paragraph before.
- **Text**: the changed middle of a paragraph is deleted and inserted (typing is a character
  insert; a 5,000-character paragraph sends a few dozen bytes per keystroke), each inserted run
  with exactly its attributes; formatting changed on text that stayed is Yjs `format` with the
  attributes that changed (`null` for those cleared).
- **Paragraph properties** change only the `pPr.*` attributes that changed.
- **Tables**: rows and cells pair up the same way; cells' blocks as stories.
- **Stories, comments, styles, lists, settings, media, files**: by key, only what changed.

Remote changes are mapped back incrementally: unchanged top-level blocks keep their objects (and
the layout's caches); only paragraphs and tables the update touched, and the stories, styles or
settings it changed, are read again.

### Awareness

Each peer's state is `{user: {id, name, color}, selection?}`: `selection` = `{story: "body" | "<story
id>", anchor, focus}`, `anchor` and `focus` Yjs relative positions (`StickyIndex::encode_v1`),
base64, in the XmlText of the paragraph they're in (`focus` is the caret). WordCraft sends its
state at most 20 times a second and renews it every 15 s; a peer not heard from for 30 s is gone.
The local caret and selection are kept as relative positions while remote changes come in, so
they stay with their text.

## Behaviour

- **Push channel.** Every running add-in's standard output is read all the time; what an add-in
  sends on its own (notifications, `collab/snapshot` and `host/log` requests) waits in a queue
  that the app's main loop takes each frame, never in the middle of a command. `host/…` requests
  during one of WordCraft's requests are still answered by it (protocol 1.1). Lines may be
  64 MiB; an add-in sending more than 1,000 messages a second for 5 s is stopped and marked
  failed. `initialize` lists `push` and `collab` in `host.features` and gives `host.collab`
  `{formats: ["yjs-v1"], app: "wordcraft", schema: 1, encryption: ["aes-256-gcm"]}`; a manifest's
  `collab {transport, label}` marks a transport.
- **Joining** (`mode: join`): `collab/joined`, then `collab/sync1`; the document is empty and
  read-only ("Joining…": commands that change it are refused) until a state for WordCraft
  schema 1 arrives. **Sharing**: the room is seeded from the document (origin `seed`), then
  `collab/joined` and `collab/sync1`. The local undo history starts afresh either way.
- **Sync.** A received `sync1` is answered with `sync2` addressed to the asker (`to` = its
  `from`); when its state vector shows changes WordCraft lacks, WordCraft asks back with its own
  `sync1`. `sync1` also goes out when the transport reports `online` after another state. An
  update that skips some of a peer's changes waits until they arrive (and WordCraft asks for
  them).
- **Remote changes** are applied between frames, the layout refreshed. A message that can't be
  decoded or applied is dropped and shown in the status bar; an update that writes another app's
  `meta` is dropped and says "This room is for <app> <schema>". Nothing a peer sends can crash
  WordCraft (panics in the Yjs library are caught too).
- **Encryption** (a `key`, 32 bytes): AES-256-GCM per protocol 2 section 4; `sync2` with
  `snapshot: true` and the answer to `collab/snapshot` use the method name `collab/snapshot`.
  After 10 messages that don't decrypt, the status bar asks "Wrong room key?".
- **`collab/snapshot`** is answered with the whole state (encrypted when keyed) and, with
  `file: true`, the document as file bytes (`file`, with `name` and `format`), except in an
  encrypted room: there WordCraft answers `fileRefused` instead, so the plaintext never reaches
  the transport. `format` (an extension, e.g. `odt`; craft-live's nextcloud-live add-in passes
  the Nextcloud file's own) picks the format when WordCraft writes it: `docx`, `docm`, `dotx`,
  `dotm`, `odt` or `rtf`. Anything else, or none, gives `.docx`; the name is the document's with
  that extension (`Notes.odt` → `Notes.docx`).
- **Offline.** When the transport reports `offline` or exits, editing goes on; an exited
  transport is started again after 3 s and the room rejoins with `sync1`, which merges both
  sides' edits.
- **Recovery.** Every 30 s the desktop app writes the room's whole state into
  `<settings folder>/live-sessions/room-<hash>.json`; joining that room again (`mode: join`) merges it
  first, so offline edits made before a crash aren't lost. Leaving deletes the file.
- **Another document**: opening or making another document ends the session (the transport is
  told).
- **Viewers** (`role: view`) can't edit; anything that changes their document goes back to the
  room's state.
- **Limits.** An update is at most 16 MiB before base64 (a larger local change is announced with
  `sync1` so peers fetch it; a larger remote one is refused), awareness 64 KiB, one room per
  document.

## In Nextcloud Office (craft-live)

[craft-live](https://github.com/sirmmo/craft-live) is a WOPI editor that Nextcloud Office uses in
place of Collabora Online. It opens documents in **WordCraft's web build**, inside Nextcloud's
page, with live co-editing; the desktop app joins the same rooms through craft-live's
nextcloud-live transport add-in. The web build implements craft-live's embed contract (its
`docs/design.md` §9), the same in GridCraft and DeckCraft. The code is
`apps/wordcraft-web/src/live.rs` (the protocol, without I/O, tested natively) and
`apps/wordcraft-web/src/web/live_io.rs` (the browser's WebSocket, `fetch` and `postMessage`).

**Starting.** craft-live frames the build as
`index.html#craft-live=1&ws=…&room=…&token=…&user=…&role=…&seed=…&file=…&name=…&format=…&api=…&lang=…`.
With that fragment the app posts `craft-live:hello` to the page around it (the shell, same
origin) as soon as it runs, and takes `craft-live:config` when the fragment can't be used. It
starts on the document, not on File › Home, with no leave-page prompt (the room keeps every
edit), in Nextcloud's language when WordCraft has it (`lang`, `pt_BR` → `pt-br`).

**Seeding and joining.** The document is read-only ("Joining…") until it's there.

- `seed=1` (the first editor): the file is fetched (`file`, `Authorization: Bearer <token>`),
  opened as File › Open does (a 0-byte file is a new, empty document) and shared with
  `collab.share` semantics (one transaction, origin `seed`), then the WebSocket connects.
- `seed=0`: the app joins and waits for the room's document (the first sync step 2; while the
  room is being seeded craft-live holds the joiner's sync step 1).
- `welcome` refusing a seed (the lease went to someone else, or the room already has a
  document) turns it into a join: merging a second seed would duplicate the content. `reseed`
  (the seeder left first) asks `POST <api>/prepare`; with `{seed: true}` the app loads the file
  and seeds on a new connection.

**The room.** `<ws>?token=<token>`, binary frames in lib0 encoding (protocol 2, Appendix B):
`0/0` sync step 1, `0/1` sync step 2, `0/2` update, `1` awareness, `2/0` permission denied,
`100` craft-live's control messages (JSON). The engine's room is joined through the transport id
`craft-live`, served in memory, so its `collab/sync1|sync2|update|awareness` map one to one onto
frames: one update per committed command, awareness (selection, chat typing) at most 20 a
second. The app sends `hello {clientId, can: ["upload"]}` when the socket opens, and holds every
other frame until `welcome`. Control messages:

| Message | What WordCraft does |
|---|---|
| `welcome` | The session starts: queued frames go, the Share button's dot turns green |
| `peers` | The people for the shell (`craft-live:peers`, with yourself, `self: true`) and the session popover; the circles in the title bar come from awareness |
| `save {id, url, name, format}` | The engine writes the file (`collab/snapshot {file: true, format}`: the file's own format when WordCraft writes it, else `.docx`, which craft-live puts in a copy beside the file) and it's uploaded: `PUT <url>` with the bearer token and `X-Craft-File-Name`. A failed upload answers `saveResult {id, ok: false, error}`. Editing goes on meanwhile |
| `saved`, `saveFailed`, `modified`, `seeded` | The status bar ("Saved to Nextcloud"), `craft-live:modified` and, after a save the person asked for, `craft-live:saved {success}`. A room with every edit saved marks the document saved |
| `notice` | The status bar; warnings and errors stay in the session's status. After a save that went to a copy, Save sends `saveNow {force: true}` (writes over the file) |
| `expired` (close 4401) | "The editing session expired. Close the document and open it again from Nextcloud." No reconnection |
| close 4403 | craft-live refused the connection: an error (the shell shows it when the document never opened) |
| any other close | Offline (red dot; editing goes on), then reconnect after 1, 2, 4, 8, 16 and then every 30 s; the sync exchange merges both sides |

**In the interface.** File › Save (Ctrl+S, the Quick Access Toolbar, Backstage) asks the room to
save into Nextcloud (`saveNow`); Save As still downloads a copy. File › Close (Ctrl+W), a
**Close** button at the right end of the title bar and Close in the session popover (in place of
Leave Live Session) send `craft-live:close`: the shell tells Nextcloud (`UI_Close`) and the app
leaves the room (`bye`). `role=view` is read-only (Save says "This document is view only"). The
people circles, the session popover and the chat panel work as in any live session.

**To the shell** (`postMessage`, `targetOrigin` = our origin): `craft-live:hello`, `loaded` (the
document is shown), `modified {modified}`, `saved {success}`, `peers {peers}`, `close`, `error
{message}` (it can't be opened: the file didn't load or the connection was refused). **From the
shell** (only from the parent window, only same-origin): `config`, `save` (Nextcloud asked: the
shell already told the server; the status bar says "Saving to Nextcloud…"), `close` (leave the
room), `getPeers`; `host` (Nextcloud's other PostMessages) is ignored.

**Building it for craft-live.** `trunk build --release` in `apps/wordcraft-web` (or
`packaging/web/package.sh`); point craft-live's `[apps] wordcraft` at the output folder
(`dist/web`), which it serves as `/apps/wordcraft/` (see `packaging/web/README.md`).

**Not verified yet** (needs a real browser and a Nextcloud with craft-live): the WebSocket,
`fetch` and `postMessage` glue in a browser (compiled for wasm, not run); Nextcloud's own
reactions to the shell's messages (craft-live's §12). The protocol is tested natively against a
stand-in room, and end to end against the craft-live server and its fake Nextcloud with the
browser's I/O played by tokio (seeding, joining, typing both ways, chat, Save and autosave
through WOPI PutFile, `.odt` saved as `.odt`, a viewer, closing, reconnecting).

## Document chat

The people editing a document talk in a chat panel beside it ([craft-collab
`docs/chat.md`](https://github.com/sirmmo/craft-collab/blob/main/docs/chat.md), protocol 2.1, the
same in GridCraft and DeckCraft). The chat **belongs to the document**: it's shared live through
the room, saved inside the file, and stays until someone clears it. Outside a live session it
works too, as notes left in the file.

**In the interface.** The title bar's chat button sits between Share and the people circles,
with the number of messages others sent while the panel was closed (a mention of you, `@Your
Name`, also makes it flash). It shows in a session, and outside one when the file has a chat;
View › Show › Chat opens the panel at any time. The panel (at the right, where task panes go)
lists the messages oldest first: the author's circle and colour, name, time (relative for today:
"just now", "5 min ago", "3 h ago"; older ones as date and time, local in the browser, UTC on
the desktop), the text (selectable; links clickable; `@Name` highlighted when it names someone
in the room or the chat), the message it answers quoted, and "edited" / "Message deleted"
markers. Right-click a message to Reply, Copy, or (your own) Edit or Delete. Who's typing shows
under the list ("Bo is typing…"). The box at the bottom sends with Enter; Shift+Enter starts a
new line. The panel's menu has **Export** (copies the chat as text), **Clear chat…** (asks
"Remove the whole chat from the document for everyone?") and the note "Kept in the file; Office
drops it when it saves".

**Commands** (everything is a command; programmatic calls never ask):

| Command | Params | Does |
|---|---|---|
| `chat.list` | `{since?, limit?}` | `{messages, clearedAt?, count}`, newest last: the messages after the one whose id is `since` (or, with a number, sent after that time in ms), the newest `limit` of them |
| `chat.send` | `{text, replyTo?}` | `{id}`. 1–4,000 characters (UTF-16 units), newlines allowed |
| `chat.edit` | `{id, text}` | The author only (their user id), not a deleted message |
| `chat.delete` | `{id}` | The author only; the entry stays as "Message deleted" |
| `chat.clear` | `{}` | `{removed}`: every message goes, for everyone; `clearedAt` is set |
| `chat.export` | `{format?: text\|json}` | The chat as text (`[time] Name: text`) or JSON |
| `chat.typing` | `{typing}` | Tells the others you're typing (awareness `typing: true`) |
| `view.chat` | `{value?}` | Shows or hides the panel |

Refused, with nothing sent: empty or too long text, a `replyTo` or `id` not in the chat, someone
else's message, a full chat (10,000 messages: "The chat is full: clear it to go on"; a deleted
message still counts, clearing frees it). The author is the room's user in a session, the app's
own user (File › Options › General name, the live-session id and colour) outside one.

**Undo and saving.** Chat changes are never undo steps, and Undo never removes a message (in a
session the chat is outside the per-person undo's scope; outside one Undo and Redo keep the chat
as it is). They do mark the document modified: it has to be saved for the chat to be kept.

**In the shared document.** A root `chat` (`Y.Array`) of message maps `{id, author (JSON text
{id, name, color}), at, text, replyTo?, edited?, deleted?}` and a root map `chatMeta`
(`clearedAt`), as craft-collab's `docs/chat.md` says; messages are only appended, so concurrent
sends land in the same order everywhere. Readers skip malformed entries (not a map, a bad id,
author, time or text, a duplicated id) and keep the first 10,000. Sharing seeds `chat` from the
file's chat; joining brings the room's chat into the document; others' messages mark it modified.
In an encrypted room the chat travels encrypted like every other update. The awareness state
carries `typing: true` while you write in the box (it goes after a 5-second pause, or when you
send).

**In the file.** DOCX (and `.docm`, `.dotx`, `.dotm`, Flat OPC): the part `/craft/chat.json`,
content type `application/vnd.craft.chat+json`, related from the package root with relationship
type `urn:craft:relationships:chat`: `{"version": 1, "clearedAt"?, "messages": [...]}`. It's
modelled (`Document::chat`), never kept as an opaque part. An empty chat writes no part; a
malformed part is skipped with a warning and the document opens. WordCraft's native JSON keeps
the same object under `chat`. Microsoft Word doesn't know the part and drops it when it saves;
the other formats (RTF, ODT, HTML, `.doc`…) don't carry the chat.

## Tests

- `crates/collab/src/tests.rs`: sharing and joining (tables, headers, objects), typing as small
  updates and concurrent typing in one paragraph merging, concurrent bold and italic, Enter,
  comments, footnotes and lists added at once without colliding ids, positions staying with their
  text, per-user undo with typing as one step, held updates, another app's room refused; and a
  fuzz where two and three replicas make random edits (typing with emoji and accents, deletes
  across paragraphs, Enter, formatting, alignment, tables, bookmarks, block removal, headers,
  footnotes, comments, undo and redo) with updates delivered out of order and twice, which must
  converge and equal what a newcomer builds (`WORDCRAFT_COLLAB_SEEDS`, `WORDCRAFT_COLLAB_STEPS`,
  `WORDCRAFT_COLLAB_SEED`; `WORDCRAFT_COLLAB_CHECK` also checks every peer's document against its
  shared state after each step).
- `crates/engine/tests/collab_convergence.rs`: two and three sessions run random command sequences
  on the sample document (typing, often in the same paragraph, Enter, Backspace, Delete, bold,
  italic, sizes, alignment, tables, comments, footnotes, cut and paste, per-user undo and redo)
  with updates in random order and duplicated; they end identical, and a newcomer gets the same.
- `crates/engine/tests/collab_conformance.rs`: replays craft-collab's WordCraft schema-1
  transcripts (`CRAFT_COLLAB_DIR`, or a `craft-collab-wordcraft` or `craft-collab` checkout next to
  this one): joining, sharing, an encrypted room, bad input, offline with concurrent typing.
- `crates/engine/tests/collab_push.rs`: a fake transport (the test binary) relays between two
  sessions through the push channel: `host.collab`, joining from an add-in command's result,
  sync, typing both ways in one paragraph, `collab/snapshot` with the `.docx`, awareness, undo,
  leaving.
- `crates/engine/tests/collab_loopback.rs`: two WordCraft processes through craft-collab's
  loopback transport (`CRAFT_COLLAB_LOOPBACK` = the built plugin).
- Chat: `crates/doc/src/chat.rs` (validation, limits, the JSON form, `chat.list`),
  `crates/docx/tests/chat.rs` (the part round trips modelled, an empty chat writes none, a
  malformed part is ignored), `crates/engine/tests/collab_chat.rs` (the commands outside a
  session, refusals, the 10,000-message limit, undo leaving the chat alone, saving to DOCX and
  the native JSON; two and three people chatting concurrently with replies, edits, deletes and
  typing mixed in, converging and a newcomer getting the same chat; sharing seeding the room from
  the file's chat and joining bringing it in; clearing; an encrypted room with nothing readable on
  the wire; typing in awareness), craft-collab's `wordcraft-1-chat` and `wordcraft-1-chat-share`
  transcripts in the conformance replay, and `crates/ui-egui/tests/chat_panel.rs` (the button,
  the panel, Enter and Shift+Enter, the menu and its confirmation, the unread count, the mention
  flash, who's typing).
- `apps/wordcraft-web/src/live/tests.rs` (craft-live mode): the lib0 framing and hostile frames,
  the fragment and `craft-live:config`, and the whole client against a stand-in for craft-live's
  room (a plain Yjs document): seeding and joining, a joiner read-only until the seed arrives,
  edits meeting, Save uploading the file in its own format (`.odt`; `.doc` brings a `.docx`),
  a seeder leaving (`reseed`), a seed the room doesn't want becoming a join, reconnecting with
  backoff and merging offline edits, expiry and refusal, a viewer, closing, the shell's requests,
  hostile control messages. `apps/wordcraft-web/tests/live_wasm.rs` runs the framing and a seeding
  session compiled for wasm (wasm-bindgen-test, in Node: `cargo test --target
  wasm32-unknown-unknown -p wordcraft-web --test live_wasm --no-run`, then
  `wasm-bindgen-test-runner` on the built `.wasm`).
- `crates/engine/tests/collab_snapshot.rs`: `collab/snapshot`'s `format` hint and the names.
- `crates/ui-egui/tests/live_session.rs`: the Share button without and with a transport, the
  Share dialog, a session through a fake transport with the state on Share and people's circles
  in the title bar, the session popover's Copy Invitation and Leave.

## Not yet

- Two people formatting the very end of a range while another types there: Yjs decides whether
  the new character takes the formatting (both copies agree).
- Styles, list definitions and document settings are whole JSON values: two people changing the
  same style at once keep the later. `revisions` is a list: two people tracking changes at the
  same moment can see one new revision labelled with the other's name until the next change.
- Comment text parts no comment refers to aren't shared.
- Typing inside an equation shares the whole equation object (last writer wins).
- The web build has no add-ins: it joins rooms only through craft-live (the y-websocket binding,
  natively). A plain y-websocket server without craft-live's control messages isn't supported yet.
- Inside craft-live, File › New and Open replace the Nextcloud document (the session ends);
  other files open from Nextcloud.
- yrs garbage collection is off (it crashed with the undo manager in GridCraft's fuzzer): deleted
  content stays in the document's history.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | minor | craft-live mode: the web build edits Nextcloud files live (craft-live's embed contract: the fragment and `craft-live:*` messages, seeding and joining, the y-websocket binding with control messages, saving by upload, reconnecting); `collab/snapshot` takes a `format` hint; Save and Close belong to Nextcloud inside craft-live |
| 2026-10-16 | minor | Document chat (craft-collab `docs/chat.md`, protocol 2.1): the shared `chat`/`chatMeta`, the `chat.*` commands and `view.chat`, the DOCX part `/craft/chat.json`, the panel and the title bar's chat button with its unread count |
| 2026-10-15 | major | Created: plugin protocol 2 (2.0.1) in WordCraft: push channel, rooms, schema 1 (Yjs XML stories, text with formatting attributes, tables, story and comment maps, per-client id blocks), encryption, per-user undo, awareness with relative positions, recovery; Share and the session's people in the title bar |
