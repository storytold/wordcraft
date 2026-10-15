# Add-ins (plugins)

> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-15 · **Change:** minor (protocol 1.2: dialogs WordCraft draws, opening local files, document events, `context.document`; previously minor: ribbon buttons that run macros: `macros/run` gets the button; previously minor: protocol 1.1: host requests, VBA macros through an add-in, the Developer tab's Code group; Office add-ins; previously major: first version: out-of-process add-ins, protocol 1, File › Options › Add-ins, install from GitHub) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

An add-in adds **commands** that people and agents run, and can put them on the ribbon (a group
on a built-in tab, or a tab of its own) and in the right-click menu. It's a separate program,
written in any language, that WordCraft starts and talks to over its standard input and output.
This is WordCraft's equivalent of Word's add-ins (Office add-ins, COM add-ins); VBA stays out of
scope.

Because an add-in runs in its own process, a crash, a hang or a bad answer fails its command,
never the document. The host's code is `crates/plugin`; the reference add-in is
`crates/plugin/src/bin/example_plugin.rs`.

**One add-in, several apps.** The manifest and the protocol are the ones GridCraft speaks, so one
add-in can serve both: `apps` names the apps it's for, `uiFor` gives each app its own ribbon and
menus, and `initialize` tells the add-in which app started it. GridCraft add-ins can also add
spreadsheet functions, which WordCraft lists and doesn't call. A test add-in written in Python
that works in both is [sirmmo/gridcraft-python-plugin](https://github.com/sirmmo/gridcraft-python-plugin).

## Installing an add-in

**From GitHub.** In **File › Options › Add-ins** (or **Insert › Add-ins › Get Add-ins**, or
**Developer › Add-ins**), type the repository's address in **Install from GitHub** and confirm.
WordCraft downloads it in the background, unpacks it into the add-ins folder of the settings (as
`<add-in id>/`, replacing an earlier install of the same add-in) and loads it. Agents and the CLI
use `plugins.install {"url": …, "dir": …}`. Addresses it understands:

| Address | Installs |
|---|---|
| `https://github.com/owner/repo` (or `github.com/owner/repo`, `owner/repo`) | The latest release; the default branch's source when there is no release |
| `…/releases/tag/v1.2.0` | That release |
| `…/tree/<branch, tag or commit>` | That source |

From a release, WordCraft takes the `.zip` asset built for this computer when there is one (see
[Publishing an add-in](#publishing-an-add-in-on-github)), else the release's source code. The
add-in is the folder in the zip that holds `plugin.json` (the shallowest one). Downloads are
limited to 256 MiB and 512 MiB unpacked; paths that would leave the add-in's folder and symbolic
links are skipped. GitHub allows 60 anonymous requests an hour per address; private repositories
can't be installed. `.craft-source.json` in the add-in's folder records where it came from, and
Options shows the repository. An add-in made only for other apps is refused.

**From a folder.** An add-in is a folder holding a `plugin.json` manifest and whatever the add-in
needs to run. WordCraft loads add-ins from:

1. each folder in `WORDCRAFT_PLUGIN_PATH` (separated like `PATH`);
2. `--plugin DIR` on the command line (`wordcraft` and every `wordcraft-cli` command; repeatable);
3. (desktop app only) the `plugins` folder in the settings folder: `~/.config/wordcraft/plugins`
   on Linux and BSD, `~/Library/Application Support/WordCraft/plugins` on macOS,
   `%APPDATA%\WordCraft\plugins` on Windows.

A folder holding `plugin.json` is one add-in. Otherwise, each of its subfolders that holds one is
an add-in. Loading only reads the manifest; the add-in's process starts the first time one of its
commands runs. **Load Add-in…** in Options loads one from its `plugin.json`.

**File › Options › Add-ins** lists the loaded add-ins with their version, author and status.
**Documentation** opens an add-in's web page (its `homepage`), **Deactivate** turns it off (it
stays listed, but its commands, ribbon buttons and menu items are gone and its process stops) and
**Activate** brings it back. WordCraft remembers which add-ins are off (`disabledPlugins` in the
editing preferences in `ui.json`). **Unload** removes it until the next start. The Add-ins dialog
(Insert › Add-ins › Get Add-ins, Developer › Add-ins) shows the same, with a button per command.

| Command | Params | Does |
|---|---|---|
| `plugins.list` (also `addins.get`) | `{}` | The loaded add-ins with their status, commands and ribbon and menu items |
| `plugins.load` | `{path}` | Loads the add-in in a folder (or a manifest file). One with the same id is replaced |
| `plugins.install` | `{url, dir?}` | Installs the add-in in a GitHub repository into `dir` (default: the app's add-ins folder) and loads it |
| `plugins.unload` | `{id}` | Unloads an add-in and stops its process (until the next start) |
| `plugins.setEnabled` | `{id, enabled}` | Turns an add-in off or back on, and remembers it in the preferences |
| `plugins.openHomepage` | `{id}` | Opens the add-in's web page in the browser |
| `plugins.run` | `{plugin, command, params?}` | Runs an add-in command (one undoable step) |

```sh
wordcraft-example-plugin --manifest > ~/addins/example/plugin.json
wordcraft-cli --plugin ~/addins/example run --cmd 'text.insert={"text":"quiet words"}' --cmd select.all \
  --cmd 'plugins.run={"plugin":"org.wordcraft.example","command":"upperCase"}' --save loud.docx
```

**Trust.** An add-in is a program running with your permissions, like any app you install.
Install add-ins only from people you trust; WordCraft doesn't sandbox them. While automation is
confined to folders (`wordcraft-cli mcp --automation-read-root … --automation-write-root …`),
agents can't load, install or turn on add-ins: a program would reach past the roots. Add-ins
loaded with `--plugin` before that keep working.

## The manifest

```json
{
  "id": "org.example.style-tools",
  "name": "Style tools",
  "version": "1.2.0",
  "description": "Tidies spacing and quotes",
  "author": "Example Corp",
  "homepage": "https://example.org/style-tools",
  "apps": ["wordcraft", "gridcraft"],
  "run": {"command": "python3", "args": ["main.py"]},
  "commands": [
    {"id": "tidy", "label": "Tidy Up", "description": "Fixes double spaces and straight quotes", "params": "{quotes?: bool}"}
  ],
  "ui": {
    "menus": [{"menu": "cell", "command": "tidy"}]
  },
  "uiFor": {
    "wordcraft": {
      "ribbon": [
        {"tab": "Style Tools", "group": "Clean", "buttons": [
          {"command": "tidy", "icon": "clear"},
          {"command": "tidy", "label": "Spaces Only", "size": "small", "params": {"quotes": false}}
        ]},
        {"tab": "Review", "group": "Style Tools", "buttons": [{"command": "tidy", "icon": "clear"}]}
      ],
      "menus": [
        {"command": "tidy"},
        {"menu": "table", "command": "tidy", "label": "Tidy Cell"}
      ]
    }
  }
}
```

| Field | Meaning |
|---|---|
| `id` | Unique id: 1–128 letters, digits, `.`, `-`, `_`, starting with a letter or digit (it's also the add-in's folder). Reverse-domain style is recommended |
| `author`, `homepage` | Shown in Options › Add-ins. `homepage` is an `http://` or `https://` link to the add-in's documentation |
| `apps` | The apps the add-in is for (`wordcraft`, `gridcraft`; any case). Left out: any app |
| `run.command`, `run.args` | The program to start. A command containing `/` or `\` is relative to the add-in folder; a bare name is looked up on `PATH`. It runs in the add-in folder, with `WORDCRAFT_PLUGIN_DIR`, `CRAFT_PLUGIN_DIR` (the folder), `CRAFT_HOST` (`wordcraft`) and `WORDCRAFT_VERSION` set |
| `commands[]` | `id` (unique in the add-in), `label`, `description`, `params` (documentation) |
| `functions[]` | Spreadsheet functions, for GridCraft. WordCraft ignores them |
| `ui` | Ribbon groups and menu items, for every app the add-in serves… |
| `uiFor.<app>` | …or for one app: `uiFor.wordcraft` is used instead of `ui` when it's there |
| `ui.ribbon[]` | A group of buttons: `tab`, `group`, `buttons`. A built-in tab's English name (`Home`, `Insert`, `Draw`, `Design`, `Layout`, `References`, `Mailings`, `Review`, `View`, `Developer`, `Help`) adds the group at the end of that tab; any other name makes a new tab after the built-in ones, shared by the add-ins that name it |
| `buttons[]` | `command` (one of the add-in's `commands[].id`), `label` and `tooltip` (default: the command's label and description), `icon`, `size` (`large`, the default, or `small`: stacked three high), `params` (passed to the command) |
| `events` | The document events the add-in hears (protocol 1.2): `documentOpened`, `documentSaved`, `documentClosed`. Names WordCraft doesn't know are ignored; at most 64 |
| `ui.menus[]` | A right-click menu item: `menu` (`text`, the default, always shown; `table` in a table; `picture` with a picture, shape or text box selected), `command`, `label`, `params`. Items go at the end of the menu, after a separator |

Icons are named from WordCraft's own icon set (`crates/ui-egui/src/icons.rs`), for example
`addins` (the default), `case`, `clear`, `wordCount`, `comment`, `link`, `table`, `picture`,
`find`, `replace`, `spelling`, `translate`, `bookmark`, `pencil`. An unknown name shows its first
letter in a tile. Add-ins can't ship their own icon images yet. Labels are at most 64 characters.
Add-in labels show as written (they aren't translated).

## Protocol (version 1)

JSON-RPC 2.0, **one JSON message per line**, UTF-8. WordCraft writes requests to the add-in's
standard input; the add-in writes one response per request to standard output. Lines on standard
output that aren't JSON are logged and ignored. Standard error goes to WordCraft's log. The add-in
exits when its standard input closes, or when it receives the `shutdown` notification.

### `initialize`

Sent once, after the process starts. Answer within 10 s with any result, for example
`{"protocolVersion": 1}`.

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"host":{"id":"wordcraft","name":"WordCraft","version":"0.6.1"},"plugin":{"id":"org.example.style-tools","dir":"/home/me/addins/style-tools"}}}
```

`host.id` says which app started the add-in, so one add-in can answer each app with its own
commands.

### `commands/run`

```json
{"jsonrpc":"2.0","id":2,"method":"commands/run","params":{"command":"tidy","params":{},
  "context":{"document":{"name":"Report.docx","path":"/home/me/Report.docx","title":"Q3 report","words":1532},
             "selection":{"text":"two  spaces","empty":false,"truncated":false,"inTable":false,"anchor":{…},"focus":{…}},
             "paragraph":{"text":"It has two  spaces.","truncated":false}}}}
{"jsonrpc":"2.0","id":2,"result":{"commands":[{"command":"text.insert","params":{"text":"two spaces","raw":true}}],"message":"Fixed 1 space","result":{"any":"json"}}}
```

`context.selection.text` and `context.paragraph.text` (the paragraph the caret is in) are cut at
100,000 characters (`truncated`). The add-in answers with engine commands to run: the same
commands the CLI, MCP and control channel use (see `wordcraft-cli commands`), except `plugins.*`
and undo history (`edit.undo`, `edit.redo`, `edit.repeat`, macros). `text.insert` replaces the
selection; `"raw": true` types the text without AutoCorrect. WordCraft runs the commands in
order, as **one undoable step**: if any fails, the document goes back to how it was and the error
is shown. Repeat and a recorded macro repeat the add-in command, not its parts. `message` is shown
in the status bar, and `result` is passed back to the caller of `plugins.run`. A command may take
up to 60 s.

### Errors and failures

- An answer `{"error": {"code": …, "message": "…"}}` fails the command with that message. The
  process keeps running.
- A command that times out, a process that exits, or broken output fails the command. The process
  is stopped and started again on the next command.
- After 3 such failures the add-in is marked `failed` and its commands fail until it's reloaded,
  or turned off and on.

### Writing an add-in

Any language that reads and writes lines works. A complete Python add-in with one command:

```python
import json, sys

for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg:          # notifications: shutdown
        break
    method, params = msg["method"], msg.get("params", {})
    if method == "initialize":
        reply = {"result": {"protocolVersion": 1}}
    elif method == "commands/run" and params["command"] == "shout":
        text = params["context"]["selection"]["text"]
        reply = {"result": {"commands": [{"command": "text.insert", "params": {"text": text.upper(), "raw": True}}]}}
    else:
        reply = {"error": {"code": -32601, "message": "unknown"}}
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], **reply}), flush=True)
```

```json
{"id": "org.example.shout", "name": "Shout", "apps": ["wordcraft"], "run": {"command": "python3", "args": ["plugin.py"]},
 "commands": [{"id": "shout", "label": "Shout"}],
 "ui": {"menus": [{"command": "shout"}]}}
```

## Protocol 1.1: asking the host, and macros

The same protocol as GridCraft and DeckCraft. While WordCraft waits for an answer to one of its
requests (`commands/run`, `macros/*`), the add-in may send requests of its own; WordCraft answers
them, then goes on waiting:

| Request | Answer |
|---|---|
| `host/sync {actions}` | `{results}`: the same actions as an Office add-in's `context.sync()` (below), on the open document; an error's `data` is `{code, action}`, and a failed sync changes nothing |
| `host/vbaProject/get {}` | `{base64}`: the document's `word/vbaProject.bin`, or `null` |
| `host/vbaProject/set {base64}` | `{}`: replaces it (at most 64 MiB; signatures over the old one are dropped) |
| `host/message {text}` | `{result: 1}`: shown in the status bar |
| `host/log {text}` | `{}`: written to the log |

What the add-in changes this way is part of the running command's one undo step.

**Macros.** An add-in that declares `"macros": {"language": "VBA"}` (craft-vba,
[sirmmo/craft-vba](https://github.com/sirmmo/craft-vba)) runs the document's VBA. With one
loaded, the Developer tab gets a **Code** group: Visual Basic (the code modules in an editor),
Macros (run, edit, create), Record Macro (WordCraft records the commands, the add-in writes them
as VBA) and Macro Security. The add-in answers `macros/list`, `macros/run {name, control?}`,
`macros/modules`, `macros/saveModule {name, source}`, `macros/deleteModule {name}`,
`macros/record {steps}`. Commands: `developer.macros`, `developer.runMacro`, `developer.modules`,
`developer.saveModule`, `developer.deleteModule`, `developer.recordMacro`,
`developer.enableMacros`, `developer.macroSecurity`. A macro is one undo step; a run-time error
takes back everything it did. WordCraft's own recorded command macros (View › Macros) are separate.

**Macro security** (Developer › Macro Security): `prompt`, the default, runs a document's macros
once the user clicked **Enable Content** in the bar a document with macros opens with; `disable`
runs none; `enable` runs all. Agents and the CLI are bound by the same setting. A document with
macros saved as `.docx` (`.dotx`) is saved as `.docm` (`.dotm`).

**Ribbon buttons** (Developer › Code › Ribbon Buttons, or Add to Ribbon… in the Macros window):
buttons that run the document's macros, on a built-in tab (Home, Insert, Design, Layout,
References, Mailings, Review, View, Developer) or a tab of the document's own (Macros by default).
They are saved with the document as Word's ribbon part (`customUI/customUI14.xml`), so Word shows
them too, and a document made in Word brings its buttons along. A ribbon that holds more than
tabs, groups and buttons (pictures of its own, menus, `onLoad`) is kept unchanged: its buttons show
and run but can't be edited. The buttons show whether or not a macro add-in is loaded; a click
runs the macro under Macro Security like any other. A button runs its macro as Word does, with
the button: `macros/run` gets `control: {id, tag}`, which a VBA add-in passes as the
`IRibbonControl` argument (`Sub Go(control As IRibbonControl)`); a macro without parameters runs as
it is. Commands (the same ids, params and results as GridCraft's): `developer.ribbon`,
`developer.addRibbonButton {macro, label?, tab?, group?, image?, large?, screentip?, tag?}`,
`developer.editRibbonButton {id, …}`, `developer.removeRibbonButton {id}` (adding, editing and
removing are undoable), `developer.pressRibbonButton {id}`. `ui.dialog {name: "ribbonButtons",
macro?}` opens the window with a macro picked.

## Protocol 1.2: dialogs, opening files, document events

The same in GridCraft, WordCraft and DeckCraft (the contract craft-nextcloud is written against).
It adds to 1.1 and changes nothing that exists; `protocolVersion` stays `1`.

**Detection.** `initialize`'s `host` gains `features`: `["dialogs", "open", "events", "document"]`,
and `language`, the interface language as a BCP 47 tag (`it`, `pt-br`, `zh-hans`…; English is
`en`). An add-in checks for a feature before relying on it.

**`context.document`.** `commands/run`'s `context.document` and `macros/run`'s
`context.document` (new) hold `path` (the file the document was opened from or last saved to,
`null` for a new document never saved), `name` (its file name), `dirty` (unsaved changes), and,
as before for commands, `title` and `words`.

**`open`.** A command's answer may hold `{"open": {"path", "label"?, "readOnly"?}}`. The path
must be absolute, be a regular file and have an extension WordCraft opens (`.docx`, `.docm`,
`.dotx`, `.dotm`, `.doc`, `.dot`, `.odt`, `.rtf`, `.txt`, `.md`, `.html`, `.tex`, `.xml`,
`.json`…); otherwise the command fails with a message and its step is taken back. Run from the
interface (a ribbon button, a menu item, a dialog's button), WordCraft opens the file after the
step exactly as File › Open does: in the window, asking first about unsaved changes, with a
password prompt for an encrypted file. `label` (at most 200 characters) is shown as the document's
location: in File › Info (the file's path as its tooltip) and in the title's tooltip; it goes
when the document is saved elsewhere. `readOnly` is ignored: WordCraft has no read-only opening.
Agents and scripts can do the same with `file.open {"path", "label"}`.

**`dialog`.** A command's answer may hold a `dialog`, which WordCraft draws itself, with its own
widgets and theme, after the step. Nothing waits: the add-in has already answered. What the user
does runs another of the add-in's commands, as `plugins.run` does, and its answer may bring
another dialog (dialogs chain), an `open`, a `message` or commands.

```json
"dialog": {
  "title": "Open from Nextcloud", "kind": "list", "prompt": "Documents",
  "items": [{"id": "/Documents/Reports", "label": "Reports", "detail": "Folder", "icon": "folder"},
            {"id": "/Documents/Notes.docx", "label": "Notes.docx", "detail": "12 KB", "icon": "file"}],
  "command": "browse", "params": {"account": "me@cloud.example"}, "okLabel": "Open",
  "buttons": [{"label": "Up", "command": "browse", "params": {"path": "/"}}]
}
```

| Kind | Shows | OK runs `command` with `params` plus |
|---|---|---|
| `list` | `items` (at most 10,000; `id` at most 4,096 characters, `label` and `detail` at most 256), each with its icon (WordCraft's icon names, as for ribbon buttons; unknown names get a file icon), label and detail. A filter box (by label) when there are more than 20. `"multi": true` allows several | `"id"`: the selected item (`"ids": [...]` with `multi`). A double-click runs OK on that item; OK waits for a selection |
| `form` | `fields` (at most 16): `{id, label, value?, kind?}`, `kind` `text` (default), `secret` (a password box), `number`, `checkbox` (`value` true or false), `choice` (`options: [{id, label}]`, `value` an option's id) | `"fields": {"<id>": value}`: text, a number (or `null` when empty), true or false, the option's id. A number field that isn't a number is said so in the dialog and nothing runs |
| `message` | `prompt` (several lines, at most 4,000 characters) | `params` alone; without `command`, OK just closes |

Every kind: `title` (at most 200; empty shows the add-in's name), `prompt` (text above the
content), `okLabel` (default WordCraft's own "OK") and up to 4 `buttons` `{label, command,
params?}`, shown left of OK, running their command with their own `params` plus the selected
`id`/`ids` (for a list, when something is selected) or `fields` (for a form). Cancel, Escape and
the window's close button just close. A step that fails keeps the dialog open with its error. The
add-in's words show as given: it translates them (`host.language` says into what). Anything over a
limit is cut; a malformed dialog, or one whose OK or buttons run a command the add-in doesn't
declare in `commands`, fails the command with a message. Run by a script (`plugins.run` from the
CLI, MCP or the control channel), nothing is opened or shown: the result holds `open` and
`dialog`, checked and bounded, for the caller to act on.

A command that changes nothing (a dialog's step that only answers with the next dialog) makes no
undo step and leaves the document as saved as it was.

One answer may combine fields: `open` with a `message` (the file opens and the message shows in
the status bar), a `dialog` with `result` and `message`, commands with either.

**Document events.** An add-in whose manifest declares `events` gets these requests, after the
fact:

| Request | Params | When |
|---|---|---|
| `events/documentOpened` | `{path, name}` | A document was opened from a local file: File › Open, Recent, a file dropped on the window, an add-in's `open`, `file.open` from a script (not one opened from bytes, on the web) |
| `events/documentSaved` | `{path, name, format}` | Save or Save As to a local file succeeded (`format`: the file's extension, lower case). Not AutoSave's writes or recovery copies (`file.save {"autosave": true}` tells no one) |
| `events/documentClosed` | `{path, name}` | A document that has a file is closed: replaced by File › Open or New in its window, or the app quits with it open |

`path` is absolute. The answer is `{}` or `{"message": "Uploaded to Nextcloud"}`; WordCraft shows
the message in its status bar, and an error answer as an error there, naming the add-in.
`documentOpened` and `documentSaved` time out after 30 s, `documentClosed` after 5 s; the time is
the whole time, host requests or not. Events go to each add-in on a thread of their own: the save,
open or close never waits for them and never fails because of them. Quitting waits at most 5 s for
`documentClosed`. An event is never part of an undo step; a timeout or crash counts as one of the
add-in's failures (three and it stops). During an event an add-in may send `host/log` and
`host/message`; `host/sync` (and `host/vbaProject/*`) is refused with `AccessDenied` (the error's
`data.code`): events are notifications, not edits.

## Office add-ins

An Office add-in's `manifest.xml` (`<Host Name="Document"/>`, `https://` pages) loads as the
add-in `office.<its Id>`: its default task pane is the command `taskpane`, and each add-in command
(buttons on Office tabs — `TabHome`, `TabInsert`, `TabDraw`, `TabDesign`, `TabPageLayoutWord`,
`TabReferences`, `TabMailings`, `TabReview`, `TabView`, `TabDeveloper`, `TabHelp` — or custom tabs,
menus, the text right-click menu `ContextMenuText`) is a command of its own. Its `<Permissions>`
bound what the page may do.

The task pane is the system web view (WKWebView on macOS, WebView2 on Windows) over a right-hand
panel (`apps/wordcraft/src/office_web.rs`). Before the page's scripts run it gets
`crates/office-js/bootstrap.js` (the one hand-written script, as in the other craft apps) and
WordCraft's own Office.js built to wasm (`cargo xtask office-js`; the macOS package carries it in
`Resources/office-js`, or set `WORDCRAFT_OFFICE_JS`). Microsoft's `office.js` is never loaded,
read or shipped. Each `context.sync()` becomes one `office.sync` command: one undo step, all or
nothing (a sync that only reads makes none).

**Objects** (Word JavaScript API; reported as WordApi 1.3, with only these objects):

| Object | Properties | Methods |
|---|---|---|
| `document` | `name` (r) | `body`, `paragraphs`, `properties`, `getSelection()`, `save()` |
| `DocumentProperties` | `title`, `subject`, `author`, `keywords`, `comments` | |
| `Body` | `text` (r), `style` | `paragraphs`, `font`, `getRange(Whole/Start/End)`, `insertText(text, Start/End/Replace)`, `insertParagraph(text, Start/End)`, `clear()`, `search(text, {matchCase, matchWholeWord})` |
| `ParagraphCollection` | `items` | `getFirst()`, `getLast()` |
| `Paragraph` | `text` (r), `style`, `styleBuiltIn`, `alignment`, `firstLineIndent`, `leftIndent`, `rightIndent`, `lineSpacing`, `spaceAfter`, `spaceBefore`, `isListItem` (r) | `font`, `getRange()`, `getNext()`, `getPrevious()`, `insertText`, `insertParagraph(text, Before/After)`, `delete()`, `select()` |
| `Range` | `text` (r), `isEmpty` (r), `style` | `font`, `paragraphs`, `insertText(text, Start/End/Before/After/Replace)`, `insertParagraph`, `delete()`, `select(Select/Start/End)` |
| `Font` | `bold`, `italic`, `underline`, `size`, `name`, `color`, `highlightColor` (`null` when the text is mixed) | |

The common API: `Office.onReady`, `getSelectedDataAsync`/`setSelectedDataAsync` (text), `settings`
(kept per open document until it closes), `requirements.isSetSupported`.

Word's API has no ids for paragraphs and ranges; WordCraft hands out opaque ones (`p:<path>`,
`r:<path>:<from>-<path>:<to>`), kept right across the edits of one sync and good until the
document's structure changes after it.

**Not yet:** content controls, inline pictures, tables, lists, sections, headers and footers,
HTML/OOXML in and out, events, dialogs, custom properties, a web view on Linux.

## Publishing an add-in on GitHub

Put `plugin.json` at the root of the repository (or in one folder of it). An add-in written in a
scripting language (`"run": {"command": "python3", "args": ["main.py"]}`) needs nothing more:
WordCraft installs the source.

A compiled add-in attaches one `.zip` per platform to each GitHub release, holding `plugin.json`
and the program (`"run": {"command": "./my-addin"}`; on Windows `./my-addin.exe`). Name each zip
after its platform and architecture, separated by `-`, `.` or `+`:

| Platform | Words WordCraft recognises |
|---|---|
| macOS | `macos`, `darwin`, `mac`, `osx`, `apple`; `universal` for both architectures |
| Windows | `windows`, `win`, `win64`, `win32` |
| Linux | `linux` |
| FreeBSD | `freebsd` |
| Apple silicon, ARM | `aarch64`, `arm64` |
| Intel, AMD | `x86_64`, `x64`, `amd64` |

For example `style-tools-1.2.0-macos-aarch64.zip`, `style-tools-1.2.0-windows-x64.zip`. A zip
naming no platform is used on every platform that has no zip of its own. Executable permissions
in the zip are kept.

## Limits and next steps

- Commands are synchronous: the window waits while an add-in works (up to 60 s).
- Add-ins show dialogs WordCraft draws (protocol 1.2: lists, forms, messages; no HTML), not
  panes of their own (Office add-ins can: their task panes). They hear documents open, save and
  close, not typing or the selection changing. A command sees the selection and the caret's
  paragraph in its context, and the whole document through `host/sync` (protocol 1.1); an event
  sees only the file.
- Installed add-ins aren't updated automatically; install again to update. There's no Uninstall
  button yet: unload the add-in and delete its folder.
- Ribbon groups go at the end of a tab; add-ins can't place them between built-in groups, add
  drop-downs or toggles, or ship their own icons yet.
- The web app can't start processes, so add-ins need the desktop app or the CLI.
- COM add-ins don't run; documents that use them keep their data. Office add-ins run with the
  subset of the Word JavaScript API above; VBA runs through a macro add-in (craft-vba).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | minor | Protocol 1.2 (shared with GridCraft and DeckCraft, used by craft-nextcloud): `host.features` and `host.language`; `context.document {path, name, dirty}` for commands and macros; answers with `open` (a local file, opened as File › Open does, with a location `label`) and `dialog` (list, form or message drawn by WordCraft, chaining); manifest `events` and `events/documentOpened`, `documentSaved`, `documentClosed`, sent without holding anything up |
| 2026-10-15 | minor | Ribbon buttons that run the document's macros, saved as its `customUI14.xml`; `macros/run` gets `control {id, tag}`; Developer › Ribbon Buttons and Add to Ribbon in the Macros window |
| 2026-10-14 | minor | Protocol 1.1 (`host/*` requests, one undo step), VBA macros through a macro add-in with the Developer tab's Code group and Macro Security; Office add-ins' task panes on the Word JavaScript API with WordCraft's own Office.js |
| 2026-10-14 | major | Created: out-of-process add-ins (the craft plugin protocol 1 shared with GridCraft: `apps`, `uiFor`, `host.id`), commands, ribbon groups and tabs, right-click menu items, File › Options › Add-ins, the Add-ins dialog, install from GitHub |
