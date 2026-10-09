# Control protocol

Start the app with a control port: `wordcraft --control 7981` (or `WORDCRAFT_CONTROL_PORT=7981`).
It listens on `127.0.0.1` only. Send one JSON object per line; each gets one reply line:

```json
{"id": 1, "method": "engine.execute", "params": {"command": "text.insert", "params": {"text": "Hello"}}}
{"id": 1, "ok": true, "result": {"anchor": {...}, "focus": {...}}}
```

## Keys

Every request needs a key as top-level `"key"`, except `chat.join`. The app creates a random host
key at start and writes it to `<config>/wordcraft/control-key.<instance>` (mode 0600, removed on
exit; `<instance>` is the Flatpak instance id, or `local-<port>` outside Flatpak). The MCP bridge
reads that file, or the env var `WORDCRAFT_CONTROL_KEY`. Chat members get their own key from
`chat.join {code}`. An unknown or revoked key gets `{"ok":false,"error":"unauthorized"}` and the
connection is closed. Lines over 1 MiB get `line too long` and the connection is closed. A
connection that sends no authorised request within 30 s of being accepted is closed (one overall
limit: blank lines or bad JSON do not extend it).

`{"method":"auth","key":"…"}` (or `params.key`) authenticates the connection: later requests may
omit `key`. The key is checked again on every request, so removing a member cuts the connection at once.
Only for `auth` is `params.key` read; elsewhere `params.key` is an ordinary parameter (`ui.key`).

Chat methods, answered by the server thread (never blocked by the UI): `chat.join {code}` (no key;
returns `handle`, `key` and `lang`, the chat's language), `chat.post {text}` (members only),
`chat.poll {after, wait_s<=25}`, `chat.members`, `chat.leave` (members only; the host gets
`host_cannot_leave`). All other methods go to the UI thread with the caller's identity. The UI does
not run a request after 55 s (the window did not draw): it answers
`{"ok":false,"error":"expired"}`; the server stops waiting at 58 s with
`{"ok":false,"error":"timeout"}`. Clients wait 60 s, so a request a client gave up on never runs.

Chat language: English by default; `WORDCRAFT_CHAT_LANG=pt` (in the app's environment) gives
Portuguese system lines, `from` names and member messages. Messages carry a `role` (`owner`,
`agent`, `system`); authority comes from the role, never from `from` (`OWNER`/`SYSTEM`, or
`DONO`/`SISTEMA` in Portuguese and in older logs).

With exactly one member in the chat, an owner message that mentions nobody gets that member's
handle in `mentions`. `@all` and `@todos` address every member. `@you`, `@ti`, `@me` and `@eu` are
never mentions; none of these (nor `@owner`, `@dono`) can be a handle. Agent text is stored on
one line, without control, bidi or zero-width characters.

Chat members get a filtered `engine.commands` and run commands as themselves.
EVERY member command is checked afterwards, also the ones marked as not editing: those must leave
the document and the undo stack exactly as they were; the others are rolled back with
`untracked change refused: …` unless they only made changes tracked under the member's name and
formatting from the allowed set (which the chat announces as `@x formatted: <command>`). The
owner's paragraphs keep their list membership and level, borders, shading, tabs, keep/page-break,
outline level and direction, also after a member splits them with Enter; their paragraph marks
(they format list labels) are compared too. Existing list definitions do not change. Denied
outright (among others): `para.defineNumber`, `para.defineBullet`, `para.setNumberingValue`,
`para.restartNumbering`, custom `kind` characters in `para.bullets/numbering/multilevel`, and any
change to the text of a comment whose author is not the member (comment text is shown without
markup). A member's accept/reject is announced whenever it changed the document, also for part of
a change, as `@x accepted|rejected N characters from <authors>` (characters over every story; a
paragraph mark counts one); a reject that would clear a new paragraph's mark is refused with
`rejecting new paragraphs: ask the OWNER`; resolving a comment is announced as
`@x resolved the comment by <author>`. For a member, `select.text`, `document.paragraph` and
`document.text` work on the body unless `story` is given (its own selection may be in a comment
or a header); `select.text` counts occurrences from the start. Members may also call
`document.paragraph` (read) and the gate's own `select.owner`, which copies the owner's selection
into the member's (only the paragraph at the caret when the owner has no selection) and returns
`text`, `anchor`, `focus`, `paragraphs` (`caretOnly`, `note`).

Errors come back as `{"id": …, "ok": false, "error": "message"}`. A failed command leaves the
document unchanged.

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command`, `params` | the command's result (any id from `engine.commands`, including UI commands `ui.tab`, `ui.dialog`, `ui.backstage`…) |
| `engine.commands` | — | every command: id, label, location, shortcut, params, enabled |
| `document.inspect` | `text?` | blocks (text, style, runs, lists, tables), parts, sections, selection, pages |
| `ui.inspect` | — | UI state, view, dialog, window size, page rects on screen, caret, perf |
| `ui.click` | `x`, `y`, `button?`, `count?`, `shift?`, `cmd?`, `alt?` | real pointer input at window coordinates |
| `ui.clickText` | `page` (0-based), `x`, `y` (points from the page's top-left), `count?` | click inside a page |
| `ui.move` / `ui.drag` | `x`,`y` / `x`,`y`,`toX`,`toY`,`steps?` | pointer move / drag |
| `ui.key` | `key` (`B`, `Enter`, `Left`…), `shift?`, `alt?`, `cmd?` | key press through egui (shortcuts apply) |
| `ui.text` | `text` | typed text through egui |
| `ui.screenshot` | `path?` | PNG of the window |
| `ui.render` | `path`, `page?` (1-based), `scale?` | PNG of a page, rendered by the engine |
| `ui.parity` | — | feature-catalog parity |
| `ui.resize` / `ui.focus` / `app.quit` | | window control |

Any other method name is treated as a command id: `{"method": "format.bold"}` works.

Positions are `{"story": "body" | {"part": n}, "path": [block, row, cell, block, …], "off": byteOffset}`.
