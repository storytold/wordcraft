# Control protocol

Start the app with a control port: `wordcraft --control 7981` (or `WORDCRAFT_CONTROL_PORT=7981`).
A missing or invalid port after `--control` (not 1–65535) prints an error and exits with status 2.
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
writes a new key. `WORDCRAFT_PROFILE_DIR` overrides the whole profile folder for isolated runs,
including recovery copies. See [local recovery](recovery.md). `<settings>` is the folder that holds the preferences: on Linux
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
restarted). It sends the key only to the app's own address, `127.0.0.1` (`localhost` connects
there too, never to `::1`): another loopback address such as `::1` or `127.0.0.2` is not where the
app listens, so whoever listens there gets no key (#3699). For
any other host it sends no key, says so, and the app refuses its requests: to drive an app on
another machine, forward its port to `127.0.0.1` (for example `ssh -L 7981:127.0.0.1:7981 host`)
and set `WORDCRAFT_CONTROL_KEY` to that app's key.

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command`, `params`, `doc?`, `author?` | the command's result (any id from `engine.commands`, including UI commands `ui.tab`, `ui.dialog` (`{"name": dialog, "part"?: table style part}`; `part` opens New / Modify Table Style (`newTableStyle` / `modifyTableStyle`) on that part: `wholeTable`, `headerRow`, `firstColumn`, `bandedRows`, `nwCell`… as `table.modifyStyle` names them), `ui.backstage`, `ui.zotero.*` (see `docs/zotero.md`), `ui.language` (`{"value": "auto"\|"en"\|"zh-hans"\|"zh-hant"\|"ja"\|"pt-br"\|"es"\|"uk"\|"sr"\|"sr-latn"\|"et"\|"tr"\|"ru"\|"pl"\|"de"\|"it"\|"cs"\|"hu"\|"fr"}`), `ui.theme` (`{"value": "system"\|"light"\|"dark"}`; `system` follows the OS appearance, light when it reports none), `ui.interfaceSize` (`{"value": "auto"}` or `{"percent": 50–300}`, clamped; no value reads it; `auto` keeps the system display scale, a percentage is absolute, so 100 is one screen pixel per point (#822); the result's `percent` is the size drawn at and `auto` whether it follows the system; document zoom stays `view.zoom`), `ui.animations` (`{"value": "system"\|"on"\|"off"}`; no value reads it; `system` follows the system's animation setting, which the desktop app reads on Windows, macOS, Linux and the BSDs and the web app from `prefers-reduced-motion`, and means on where the system doesn't say; the result's `enabled` is what the setting comes to and `system` what the system reports, `null` when it doesn't say), `ui.fileBrowser` (desktop: `{"purpose": "document"\|"picture"\|"recipients"\|"save"}` opens WordCraft's own file browser, for when the system file dialog doesn't show up, #576), `ui.saveAsTemplate` (File › Save As › Save as Word Template: the Save dialog for a `.dotx`, starting in the user templates folder; the result's `dir` is that folder, `null` on the web, #2852), `ui.caretAnimation` (`{"value": true\|false}`; no value reads it; off, the caret jumps and blinks on and off instead of gliding and fading), `ui.startScreen` (`{"value": true\|false}`; no value reads it; on, the default, the desktop and web apps start on File › Home when launched with nothing to open, #766), `ui.alignmentGuides` (`{"value": true\|false}`; no value reads it; on, the default, moving a picture or shape snaps it to the page, the margins and other objects, with guide lines while dragging; Alt moves freely), `ui.quickAccessToolbar` (`{"value": true\|false}`; no value toggles it; false takes the Quick Access Toolbar out of the title bar, as its menu's Hide Quick Access Toolbar does, and true brings it back, #1945), `ui.focusRibbon` (`{"value": true\|false}`; no value toggles it; in Focus, true drops the ribbon down over the page, as the bar's ⋯ does, and false puts it away, as a click on the page does; outside Focus it stays put away, #1678), `ui.contextMenu` (opens the page's context menu below the caret, as Shift+F10 does), `ui.paneDock` (`{"pane": "navigation"\|"clipboard"\|"styles"\|"styleInspector"\|"accessibility"\|"altText"\|"formatShape"\|"restrictEditing"\|"comments"\|"smartArtText", "dock"?: "left"\|"right"\|"float", "before"?: pane\|null}`; no `dock` reads it; docks or floats a task pane as its menu does, kept between runs; Styles is `view.stylesPaneDock`, #1756; `before` puts the pane directly before that pane in the order, outside it on their edge, as a drop over a docked pane's outer third does, and `null` after every pane; the order is kept between runs)…) |
| `engine.commands` | — | every command: id, label, location, shortcut, params, enabled |
| `document.inspect` | `text?`, `doc?` | blocks (text, style, runs, lists, tables), parts, sections, selection, pages |
| `ui.inspect` | — | UI state, view, dialog, window size, page rects on screen, caret, comment balloons (`balloons`: id + screen rect; `balloon`: the selected one), perf; `document`: the active document's id; `documents`: the open documents (`id`, `title`, `path`, `dirty`, `active`) |
| `ui.click` | `x`, `y`, `button?`, `count?`, `shift?`, `cmd?`, `alt?` | real pointer input at window coordinates |
| `ui.clickText` | `page` (0-based), `x`, `y` (points from the page's top-left), `count?` | click inside a page |
| `ui.move` / `ui.drag` | `x`,`y` / `x`,`y`,`toX`,`toY`,`steps?` | pointer move / drag |
| `ui.press` / `ui.release` | `x`,`y`, modifiers | primary button down / up there (with `ui.move` between: a drag you can screenshot halfway) |
| `ui.wheel` | `x`, `y`, `dy`, `dx?`, `unit?` (`point`/`line`/`page`), `phase?` (`start`/`move`/`end`), modifiers | mouse-wheel or trackpad scroll at window coordinates (`dy` < 0 scrolls down) |
| `ui.key` | `key` (`B`, `Enter`, `Left`…), `shift?`, `alt?`, `cmd?` | key press through egui (shortcuts apply) |
| `ui.text` | `text` | typed text through egui |
| `ui.screenshot` | `path?` | PNG of the window: written to `path`, or without one returned as base64 `png` in the reply |
| `ui.render` | `path?`, `page?` (1-based), `scale?`, `doc?` | PNG of a page, rendered by the engine: written to `path`, or without one returned as base64 `png` in the reply (never a fixed name in the shared temporary folder) |
| `ui.parity` | — | feature-catalog parity |
| `ui.resize` | `width`, `height` | asks for that window content size in points; sizes that aren't positive and finite are refused, the rest are clamped to the app's minimum (760 × 480) and to the largest surface the GPU can draw (at most 16384; that limit wins over the minimum), and the reply's `width` and `height` are the size asked for |
| `ui.focus` / `app.quit` | | window control |

Any other method name is treated as a command id: `{"method": "format.bold"}` works.

### Open documents

Several documents can be open at once. Each has an id, a whole number that stays the same while
the document is open and is never reused by the running app. On the desktop each open document
has a native window of its own; the web app shows them one at a time in its one window. One
document is the active one: on the desktop, the document of the window focused last (the active
window); in the browser, the one the window shows. Commands work on it unless told otherwise.

- `file.new` opens another document in a new window, focused, and makes it active. `file.open`
  does the same with a file; it replaces a blank document nobody has used yet in that document's
  window, and a file that is already open becomes active again and its window comes to the front
  (`"alreadyOpen": true`). Its result carries the document's `id`.
- `view.switchWindows` lists the open documents (`{"documents": [{"id", "title", "path",
  "dirty", "active"}]}`); with `{"id": n}` it makes that document active and brings its window to
  the front.
- `file.closeDocument {"id"?, "force"?}` closes a document; its window closes with it unless it
  is the last window.
- `engine.execute`, `document.inspect` and `ui.render` take `doc`, the id of the document to work
  on, next to their other params. The command runs on that document and the active window stays
  active. A dialog or page the command opens (Save As, Print) opens in that document's window,
  which comes to the front:

```json
{"id": 2, "key": "…", "method": "engine.execute", "params": {"command": "text.insert", "params": {"text": "Hello"}, "doc": 3}}
```

`doc` left out, or `null`, is the active document. An id that names no open document, or a value
that isn't a whole number from 0 up, is an error and nothing runs. Window commands (`ui.*`) work
on the active window only: with the id of another document they are an error.

### Signing a command

`engine.execute` takes `author`, a name that signs that one command's comments and tracked changes
instead of the document's author (File › Options › General), with initials drawn from the name. An
agent editing beside a person sets it, so Review shows its changes apart from theirs and they can
be accepted or rejected as a group. The name is trimmed and cut to 200 characters; left out, `null`
or blank, the command signs as usual, and a value that isn't a string is an error. The app signs
with its own name again once the command has run.

```json
{"id": 3, "key": "…", "method": "engine.execute", "params": {"command": "edit.replaceAll", "params": {"text": "30 days", "with": "45 days"}, "author": "Drafting assistant"}}
```

### The active window

Input and window methods go to the active window, the one focused last: `ui.click`, `ui.key`,
`ui.text`, `ui.wheel` and the other input methods (coordinates are that window's),
`ui.screenshot` (a PNG of that window), `ui.inspect` (its size), `ui.resize` and `ui.focus`. To
work in another window, make its document active first with `view.switchWindows {"id": n}`.

The `CharProps` and `ParaProps` that `format.set`, `para.set` and `styles.modify` take are listed
in [MCP server](mcp.md#character-and-paragraph-properties).

Positions are `{"story": "body" | {"part": n}, "path": [block, row, cell, block, …], "off": byteOffset}`.

### Keyboard and text input

`ui.key` and `ui.text` go to this window's focused widget even when another app is in front
(#2420). A focused Find field receives the text; a document receives it only while its canvas
owns keyboard focus. Shortcuts, dialogs and document protection still apply. These methods do
not activate the native window. `ui.focus` separately requests native activation.

The `{"queued": true}` reply acknowledges the input queue, not a completed edit. Input is applied
on subsequent UI frames. Inspect the resulting document or widget state before continuing with
an operation that depends on it; do not replay an edit simply because its effect is not immediate.

### Closing the document

`engine.execute` with `file.close` never shows the Save / Don't Save / Cancel prompt. On a
document with unsaved changes it replies with an error (the app keeps running and the edits
stay); save first (`file.save`), or pass `{"force": true}` to discard the changes and close.
The bare form `{"method": "file.close", "force": true}` (a method name that is a command id) is
guarded the same way, and only a boolean `true` counts as `force`. A document without unsaved
changes closes at once (#1268). Closing closes the document and its window, and the window
focused before it becomes the active one. The last window stays open with no page, and
`file.new` or `file.open` goes on from there (#1712); closing it doesn't quit the app. With
`doc` naming a document in the background, `file.close` closes that document and its window under
the same rule (unsaved changes need `{"force": true}`), and the active window stays active.

### Native printing

`engine.execute` can call `ui.printSettings` with `{}` to inspect the printer list, selected
printer, copies, duplex, grayscale, discovery/submission status and backend receipt/error.
`{"refresh":true}` starts background discovery; poll the same command until `discovering` is
false. Set `printer` to a discovered name, integer `copies` from 1 to 999, `duplex` to
`printerDefault`, `off`, `longEdge` or `shortEdge`, and boolean `grayscale`. Invalid updates are
rejected without changing earlier settings. `ui.printSubmit` explicitly submits the current
print-ready PDF without opening a dialog; inspect `ui.printSettings` until `busy` becomes false
and check `status`/`detail`. The browser keeps its own print hook. PDF export remains separate.

`ui.printSubmit` / `ui.print` also accept the upstream PDF page settings (`range`, `pages`,
`parity`, `copies`, `collate`). Native printing renders the selected pages once and passes
copies/collation to the spooler; the browser and `file.printPdf` keep copies in the PDF.
Omitting native `copies` uses the configured `ui.printSettings` value. `collate` defaults to
File › Print's setting; invalid ranges submit no job. `paper` (`none`, or a Layout › Size name
such as `Letter`, `Legal`, `A4`) is Scale to paper size (#2108): every page is scaled onto that
paper and centred, and native jobs ask the spooler for that `media`; the document's page size is
unchanged. `file.printPdf` takes it too; an unknown paper submits no job.

`engine.commands {"search":"print"}` lists the engine print commands and the app-owned
`ui.printSettings`, `ui.printSubmit`, and legacy `ui.print` alias, with current enablement
and their parameter documentation. Search is case-insensitive across IDs, labels, locations
and parameter descriptions. Listing commands never discovers printers, submits a job or opens
a dialog. Settings inspection is always available; submission becomes enabled when the native
printer is ready, or when the browser host provides its print flow.
