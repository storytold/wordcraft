# MCP server

WordCraft speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio, so
Claude and other agents can write and edit documents.

```sh
wordcraft-cli mcp                          # headless session (no window)
wordcraft --control 7981 &                 # or: drive the running app…
wordcraft-cli mcp --connect 127.0.0.1:7981 # …including clicks, keys and screenshots
```

Claude Code: `claude mcp add wordcraft -- wordcraft-cli mcp`.

With `--connect`, the bridge sends the app's control key, which it reads from
`WORDCRAFT_CONTROL_KEY` or from the key file the app writes at start. It sends the key only to
loopback addresses; see [Keys](control-protocol.md#keys). The bridge serves even when the app
isn't open yet: tool calls return "not reachable … will retry on each call" and succeed once the
app's port opens (#4078), so a client that starts before WordCraft doesn't lose the server for
its whole session.

## Scripts from the command line

`wordcraft-cli run --script job.json` (`--script -` reads standard input) runs the commands in a
file, in order, exactly like repeated `--cmd`, and then saves. Use it to generate a DOCX and its
PDF in one run without a window:

```json
{"template": "sample",
 "commands": [{"command": "select.text", "params": {"text": "Membership"}},
              {"command": "format.bold"},
              "caret.docEnd"],
 "save": ["out.docx", "out.pdf"]}
```

`file` (a document to open) and `template` are optional, and `--file` / `--template` on the command
line win. A command is `{"command": id, "params": {…}}` or just the id. `save` is a path or a list;
each output's format follows its extension. Relative `file` and `save` paths resolve against the
script's folder (the working folder for stdin). `--cmd` flags run after the script's commands and
`--save` after its outputs. The run starts with one open document. `file.new` and `file.open`
open another one and make it active; later commands and the saves work on the active document.
`view.switchWindows` lists the open documents (with `--print` their ids show), and
`view.switchWindows={"id": n}` makes one active. A failing command stops the run with `command N (id): …` and exit
status 1, and nothing is saved. Every output is checked (known extension, folder exists) before the first is written. Paths inside command params (a picture to insert, say) are relative to the working folder, not the script's. Scripts are capped at 4 MiB.

## Tracked changes by an agent

Tracked changes and comments are recorded under the session's user name ("WordCraft User" by
default). Name the agent so reviewers can tell its edits from people's: start the headless server
with `wordcraft-cli mcp --author "Claude (copyedit)"`, or run
`file.setAuthor {"name": "Claude (copyedit)"}`. Turn tracking on explicitly with
`review.trackChanges {"value": true}`; without `value` it toggles, which turns tracking *off* in
a document that was saved with it on. `wordcraft-cli run` takes `--author` too.

## Confining file access

```sh
wordcraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project/out
```

```json
{"mcpServers": {"wordcraft": {"command": "/abs/path/target/release/wordcraft-cli",
  "args": ["mcp", "--automation-read-root", "/work/project", "--automation-write-root", "/work/project/out"]}}}
```

The two flags (VectorCraft, PhotoCraft and the other Craft apps take the same ones, so one client
config works across the suite) keep the agent to the folders you give it:

- Every file a command reads must lie inside the **read root**: `open_document` and `file.open`,
  `file.newFromTemplate`, `insert.textFromFile` and `insert.object`, `insert.picture` and
  `picture.change` with a `path`, `mailings.recipients` with a `path` (csv, tsv, txt, xlsx, xlsm,
  ods), `review.compare` (`path` and `original`), the templates and documents `tools.updateStyles`,
  `styles.organizer` and `styles.copyFrom` read, and the folders `file.options` takes
  (`dictionariesFolder`, `userTemplatesFolder`). The pictures an opened HTML file names are read
  from beside that file and must be inside the read root too, even when the file was opened through
  a link (#3736).
- Every file a command writes must lie inside the **write root**: `save_document`, `file.save`,
  `file.saveAs`, `file.rename` (the file and its new name), `file.exportPdf`, `file.exportPng`, `file.printPdf`,
  `file.saveTemplate`, and `mailings.finish` with a `path`. `font.add` and `font.remove` change the
  user font folder, so they are refused when it is outside the write root (#3692). A save replaces
  the name it is given, so that name must be inside the write root as well as what it leads to:
  a link outside the root pointing in is refused (#3735).
- Read and write access are separate. Give the same folder twice for both; a root left out grants
  none of its access (with only `--automation-read-root`, nothing can be written to disk). Without
  either flag nothing changes.
- A path is made absolute against the server's working directory, then its links are followed: a
  file (or folder) link that leads out of a root is outside it. For a file that doesn't exist yet,
  its nearest existing folder's links are followed and the rest must be plain names, so `..` below a
  folder that doesn't exist is refused, as is a link that leads nowhere (writing through it would
  create its target). The result is compared with the root by whole folder names: `/work2/x` is not
  inside `/work`. On Windows case is ignored, UNC paths (`\\server\share\...`) are resolved like
  drive-letter ones, and device names (`CON`, `NUL`, `COM1`...) are refused.
- A refused path is an ordinary tool error the agent can read and correct, for example
  `automation path rejected: outside the write root /work/project/out: /etc/x.docx` or
  `automation filesystem access is not granted: write authority is absent: /work/project/out/x.docx`.
- The roots must be existing folders; a missing one stops the server with an error. Each flag is
  given once (`--flag <dir>` or `--flag=<dir>`).

The flags confine the headless server; `--connect` with them is an error, since the server would
otherwise talk to an app that isn't confined. WordCraft's desktop app has no equivalent flags yet,
so a running app (`wordcraft --control`) is not confined.

What the roots do not cover: commands that take no path (`text.insert`, `execute` of anything that
works on the open document) can still read and change that document, and `get_text` and
`inspect_document` return it. The Zotero commands talk to a local Zotero over a socket, not to files.
Anything that reads or writes a path goes through one place in the engine (`io::open_path_with`,
`io::save_path_with`, `io::read_file`, `io::write_file`); a test keeps command modules from
touching the file system on their own.

The checks hold for every thread of the server process. A path is checked, then opened: another
program that swaps a link in between can still win that race, so the roots keep an agent to the
files it was given rather than guarding against other software on the computer.

## Tools

| Tool | What it does |
|---|---|
| `list_commands` | every command (filter with `query`) — over 270 of them |
| `execute` | run one command: `{command: "insert.table", params: {rows: 3, cols: 4}}` |
| `batch` | run several commands in order |
| `new_document` | blank, sample, letter, resume, report; opens beside the open documents |
| `open_document` / `save_document` | docx, doc (open only), odt, rtf, html, md, tex, txt, json; save also pdf, png |
| `list_documents` | the open documents: `id`, `title`, `path`, `dirty`, `active` |
| `type_text` | type at the caret (`paragraphs: true` splits lines into paragraphs: one undo
  step; each break is the Enter it stands for, so AutoFormat and AutoCorrect run and a
  one-line control refuses; at most 10,000 lines — the cap is `text.insert`'s own, so a
  control-channel or CLI call is held to it too) |
| `select_text` | select the n-th occurrence of some text |
| `get_text` / `inspect_document` | read the document back (verify without screenshots) |
| `render_page` | a page as PNG |
| `parity` | feature coverage |
| `screenshot`, `click`, `key`, `ui_inspect` | app only (`--connect`) |

### Several documents

`new_document` and `open_document` (the commands `file.new` and `file.open`) open another
document beside the ones already open and make it the active one; their reply carries its `id`.
`open_document` replaces a blank document nobody has used, and a file that is already open becomes
active again (`alreadyOpen`). `list_documents` lists the open documents with their ids.

Tools work on the active document. `execute`, `batch`, `type_text`, `select_text`, `get_text`,
`inspect_document`, `render_page` and `save_document` also take `doc`, the id of another open
document: the tool works on that document and the active one stays active.

```json
{"name": "execute", "arguments": {"command": "text.insert", "params": {"text": "Hello"}, "doc": 3}}
```

An id that names no open document, or a value that isn't a whole number from 0 up, is a tool error.
`execute` with `view.switchWindows {"id": n}` makes a document active, and `file.closeDocument
{"id": n, "force"?: true}` closes one (unsaved changes need `force`). Through `--connect`, `doc`
reaches the app's control channel (see [Open documents](control-protocol.md#open-documents)); the
app's window keeps showing the document it showed. `screenshot`, `click` and `key` act on the
window, so they work on the document on screen.

`insert.picture` and `insert.icon` leave the caret after the new picture, so typing next doesn't
replace it; Picture Format commands (`picture.*`, `arrange.*`) still act on it.

Resources: `wordcraft://document` (inspect) and `wordcraft://commands`.

The acceptance test `crates/mcp/src/tests.rs` writes a formatted document using MCP only.

## Character and paragraph properties

`format.set {"props": CharProps}`, `para.set {"props": ParaProps}`,
`styles.modify {"style": …, "chr"?: CharProps, "para"?: ParaProps}` and the `para` of
`table.newStyle` / `table.modifyStyle` take these two objects. Every field is optional: a field
left out keeps what the style chain gives. A field name the object doesn't know is ignored, not
refused, so check the spelling. Lengths are points. A colour is a hex string `"RRGGBB"` or
`"#RRGGBB"`, or `[r, g, b]` (three numbers 0–255); it reads back as `[r, g, b]` (#865).

```json
{"command": "styles.modify", "params": {"style": "Heading1",
  "chr": {"font": "Georgia", "size": 18, "bold": true, "color": "1B1611"},
  "para": {"spaceBefore": 12, "lineSpacing": {"rule": "multiple", "value": 1.15}, "keepNext": true}}}
```

### CharProps

| Field | Type | Values |
|---|---|---|
| `style` | string | character style id |
| `font` | string | font name |
| `size` | number | points |
| `bold`, `italic`, `strike`, `doubleStrike`, `caps`, `smallCaps`, `hidden`, `outline`, `shadow`, `emboss`, `engrave`, `noProof` | bool | |
| `underline` | string | `none`, `single`, `words`, `double`, `thick`, `dotted`, `dash`, `dotDash`, `dotDotDash`, `wave`, `doubleWave` |
| `underlineColor` | colour | |
| `color` | `"Auto"`, a colour, or `{"Rgb": colour}` | `Auto` (also `"auto"`) is black on light backgrounds, white on dark; read back as `"Auto"` or `{"Rgb": …}` (#865) |
| `highlight` | string | `none`, `yellow`, `brightGreen`, `turquoise`, `pink`, `blue`, `red`, `darkBlue`, `teal`, `green`, `violet`, `darkRed`, `darkYellow`, `gray50`, `gray25`, `black`, `white` |
| `shading` | colour | background fill |
| `border` | Border | character border; `"style": "none"` is explicitly no border |
| `vertAlign` | string | `baseline`, `superscript`, `subscript` |
| `spacing` | number | character spacing, points: positive expands, negative condenses |
| `scale` | number | horizontal scale, percent (100 = normal) |
| `position` | number | raised (positive) or lowered (negative), points |
| `kern` | number | kern fonts at or above this size, points; 0 = off |
| `textEffects` | object | WordArt `fill`, `outline`, `shadow`, `glow`, `reflection` (see `crates/doc/src/wordart.rs`) |
| `lang` | string | BCP 47 language tag (proofing) |
| `rtl` | bool | right-to-left run |
| `cs` | bool | format the whole run with the complex-script properties |
| `fontCs`, `fontEa` | string | complex-script font; East Asian font |
| `sizeCs` | number | complex-script size, points |
| `boldCs`, `italicCs` | bool | complex-script bold, italic |
| `langBidi` | string | BCP 47 language of complex-script text, e.g. `fa-IR` |
| `link` | string | hyperlink target: a URL, or `#bookmark` |
| `ins`, `del`, `fmtChange` | | Track Changes bookkeeping (revision indexes) |

`format.set` also applies `size`, `bold` and `italic` to `sizeCs`, `boldCs` and `italicCs` when
those aren't given; `styles.modify` sets only what it is given.

A **Border** is `{"style": …, "width": pt, "color": colour | null, "space": pt}` (`space` is
the distance from the text). Missing fields are `none`, 0, no colour and 0, so a border needs a
`width` above 0 to show. `style` is one of `none`, `single`, `double`, `dotted`, `dashed`,
`thick`, `triple`, `dotDash`, `wave`.

### ParaProps

| Field | Type | Values |
|---|---|---|
| `style` | string | paragraph style id |
| `align` | string | `left`, `center`, `right`, `justify`, `distribute` (`left` is the start edge: the right in a right-to-left paragraph) |
| `indentLeft`, `indentRight` | number | points |
| `indentFirst` | number | first-line indent, points; negative = hanging |
| `indentLeftChars`, `indentRightChars`, `indentFirstChars` | number | the same indents in characters; a non-zero value wins over the one in points, 0 cancels an inherited one |
| `spaceBefore`, `spaceAfter` | number | points |
| `lineSpacing` | object | `{"rule": "multiple", "value": 1.15}` (1 = single, 2 = double), `{"rule": "atLeast", "value": pt}`, `{"rule": "exactly", "value": pt}` |
| `contextualSpacing` | bool | ignore space before/after between paragraphs of the same style |
| `mirrorIndents` | bool | left and right indents swap on even pages |
| `keepNext`, `keepLines`, `pageBreakBefore`, `widowControl` | bool | |
| `outlineLevel` | integer | 0–8 = Level 1–9, 9 = body text |
| `numbering` | object | `{"num": n, "level": n}`, both required; `level` counts from 0; `num` 0 removes numbering a style gives |
| `tabs` | array | `[{"pos": pt, "align"?: "left\|center\|right\|decimal\|bar\|clear", "leader"?: "none\|dot\|hyphen\|underscore\|middleDot"}]`; replaces the inherited stops, `clear` removes an inherited one at `pos` |
| `shading` | colour | background fill |
| `borders` | object | `{"top", "left", "bottom", "right", "between"}`, each a Border (`between`: between paragraphs; `inside_v` is used by tables) |
| `suppressHyphens`, `suppressLineNumbers` | bool | |
| `bidi` | bool | right-to-left paragraph |
| `dropCap` | integer | lines to drop, 0 = none |
| `kinsoku`, `wordWrap`, `overflowPunct`, `autoSpaceDE`, `autoSpaceDN`, `snapToGrid` | bool | East Asian typography; unset = on |
| `topLinePunct` | bool | East Asian typography; unset = off |
| `numChange`, `fmtChange` | | Track Changes bookkeeping |

`para.set` also takes `"onPage": {"align"?: "left|center|right|justify|distribute", "left"?: pt,
"right"?: pt}`: the alignment and indents as seen on the page, which each selected paragraph turns
into its own start and end once `props` (a `bidi` in it, say) is applied. The Paragraph dialog sends
only the fields that changed this way, as one command and so one undo step.
