# Control protocol

Start the app with a control port: `wordcraft --control 7981` (or `WORDCRAFT_CONTROL_PORT=7981`).
It listens on `127.0.0.1` only. Send one JSON object per line, with the window's key (see
[Keys](#keys)); each gets one reply line:

```json
{"id": 1, "key": "…", "method": "engine.execute", "params": {"command": "text.insert", "params": {"text": "Hello"}}}
{"id": 1, "ok": true, "result": {"anchor": {...}, "focus": {...}}}
```

Errors come back as `{"id": …, "ok": false, "error": "message"}`. A failed command leaves the
document unchanged. A line that isn't valid JSON gets one error reply, then the server closes the
connection (so an HTTP request sent to the port can't run a command); reconnect to continue.

## Keys

Any local program (and possibly a web page) can reach a loopback port, so every request needs the
window's key. Once the port is bound, the app writes a random key (256 bits, as hex) to
`<settings>/control-key.<instance>` and removes the file when the window closes; a new start
writes a new key. `<settings>` is the folder that holds the preferences: on Linux
`$XDG_CONFIG_HOME/wordcraft` (by default `~/.config/wordcraft`), on macOS
`~/Library/Application Support/WordCraft`, on Windows `%APPDATA%\WordCraft`. `<instance>` is the
Flatpak instance id inside Flatpak, and `local-<port>` otherwise. On Unix the file has mode 0600.

```sh
cat ~/.config/wordcraft/control-key.local-7981   # Linux, outside Flatpak, port 7981
```

- Send the key as top-level `"key"` with every request, or once per connection with
  `{"method": "auth", "key": "…"}` (reply `{"ok": true, "result": {}}`); later requests on that
  connection may leave `key` out. Only `auth` reads `params.key`; for other methods `params.key`
  is an ordinary parameter (`ui.key {"key": "Enter"}` presses Enter).
- The key is checked on every request. A missing or wrong key gets
  `{"ok": false, "error": "unauthorized"}`, and the app closes the connection.
- A line over 1 MiB gets `{"ok": false, "error": "line too long"}`, and the app closes the
  connection.
- A connection that sends no authorised request within 30 s of being accepted is closed. That is
  one limit: blank lines and bad JSON do not extend it. After an authorised request the
  connection may stay idle.

`wordcraft-cli mcp --connect` takes the key from `WORDCRAFT_CONTROL_KEY`, or else from the key
file for the port it connects to, and reads it again on every new connection (the app may have
restarted). It sends the key only to loopback addresses (`127.0.0.0/8`, `::1`, `localhost`). For
any other host it sends no key, says so, and the app refuses its requests: to drive an app on
another machine, forward its port to `127.0.0.1` (for example `ssh -L 7981:127.0.0.1:7981 host`)
and set `WORDCRAFT_CONTROL_KEY` to that app's key.

## Chat

Review › Chat › Start Chat opens the port (if `--control` did not) and creates the window key.
Stop Chat revokes every member at once and closes the port that Start Chat opened. See
[chat.md](chat.md).

- `chat.join {code}` needs no key. It answers `{"handle", "key"}` (a member key, 256 bits, as
  hex). A wrong, used or expired code (`invite_invalid`, `invite_expired`) closes the connection.
- A member key works like the window key (top-level `key`, or `auth`). The app checks it again on
  every request: Remove or Stop Chat cuts the member off at once (`unauthorized`).
- The server thread answers these, never waiting on the UI: `chat.poll {after, wait_s}` (messages
  with `seq > after`, waiting up to `wait_s` seconds, at most 25), `chat.members`, a member's
  `chat.post {text}`, and `chat.leave` (members only). With the window key, `chat.post` goes to
  the UI thread and runs the `chat.post` command as the owner.
- Every other method goes to the UI thread with the caller's identity. A member's request goes
  through the chat gate: only the commands on the agent allow-list run, as the member, and each
  one is checked after it runs. The UI does not run a request after 55 s (`expired`); the server
  stops waiting at 58 s (`timeout`).
- Messages: `{"seq", "ts_ms", "from", "role": "owner"|"agent"|"system", "text", "mentions"}`.
  Authority comes from `role`, never from `from`.

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command`, `params` | the command's result (any id from `engine.commands`, including UI commands `ui.tab`, `ui.dialog`, `ui.backstage`, `ui.zotero.*` (see `docs/zotero.md`), `ui.language` (`{"value": "auto"|"en"|"zh-hans"|"zh-hant"|"ja"|"pt-br"|"es"|"uk"}`), `ui.theme` (`{"value": "system"|"light"|"dark"}`; `system` follows the OS appearance, light when it reports none)…) |
| `engine.commands` | — | every command: id, label, location, shortcut, params, enabled |
| `document.inspect` | `text?` | blocks (text, style, runs, lists, tables), parts, sections, selection, pages |
| `ui.inspect` | — | UI state, view, dialog, window size, page rects on screen, caret, perf |
| `ui.click` | `x`, `y`, `button?`, `count?`, `shift?`, `cmd?`, `alt?` | real pointer input at window coordinates |
| `ui.clickText` | `page` (0-based), `x`, `y` (points from the page's top-left), `count?` | click inside a page |
| `ui.move` / `ui.drag` | `x`,`y` / `x`,`y`,`toX`,`toY`,`steps?` | pointer move / drag |
| `ui.press` / `ui.release` | `x`,`y`, modifiers | primary button down / up there (with `ui.move` between: a drag you can screenshot halfway) |
| `ui.key` | `key` (`B`, `Enter`, `Left`…), `shift?`, `alt?`, `cmd?` | key press through egui (shortcuts apply) |
| `ui.text` | `text` | typed text through egui |
| `ui.screenshot` | `path?` | PNG of the window |
| `ui.render` | `path`, `page?` (1-based), `scale?` | PNG of a page, rendered by the engine |
| `ui.parity` | — | feature-catalog parity |
| `ui.resize` / `ui.focus` / `app.quit` | | window control |

Any other method name is treated as a command id: `{"method": "format.bold"}` works.

Positions are `{"story": "body" | {"part": n}, "path": [block, row, cell, block, …], "off": byteOffset}`.
