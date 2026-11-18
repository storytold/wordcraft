# Plug-ins

WordCraft runs command plug-ins: small WebAssembly modules that work on the open document
through the same commands as the ribbon, the shortcuts, the CLI, the control channel and MCP.
They run in the sandbox of craft-libs' shared `craft-plugin` crate
([RFC-0046](https://github.com/storytold/craft-contrib/blob/main/rfcs/0046-shared-plugin-runtime.md)),
with an instruction budget and memory limits, and reach nothing but the commands below. The ABI,
the manifest (`craft-plugin.json`) and the `.craftplugin` bundle are described in the
[craft-plugin README](https://github.com/storytold/craft-libs/blob/main/crates/plugin/README.md).
A manifest must name `wordcraft` in `apps`.

## Using plug-ins

- **Insert › Add-ins › Get Add-ins** opens the Plug-ins window on getting more: from a
  `.craftplugin` file, from a GitHub repository's release (`owner/repo` or `owner/repo@tag`; the
  bundle is checked against the release's `SHA256SUMS`) or from a catalog (an `index.json` on
  GitHub, or a local `file://` one while you write an entry).
- Before anything is installed, a confirmation shows the plug-in's name, author, kind, access
  (read or edit the document), the commands it may call, where it comes from and its SHA-256. It
  says plainly when the bundle is in no catalog you use.
- **Insert › Add-ins › My Add-ins** lists each installed plug-in's commands. A command with
  parameters asks for them first, in a form made from its manifest. **Manage Plug-ins…** runs,
  updates and removes them.
- A run is **one undo step**, named after the plug-in. A run that fails (a trap, an exhausted
  budget, a non-zero status) leaves the document, the selection and the undo history as they
  were. A run makes at most 100 edits.
- The desktop app keeps plug-ins in `Plug-ins` in its settings folder, and each plug-in's
  settings (at most 64 KB, keys up to 128 characters) and the catalogs added in
  `plugin-settings.json` beside it. The CLI and MCP keep plug-ins for the session only; they
  download nothing (install from a file, `data` or a local catalog there).
- Plug-in panels (`ui_show`) aren't shown yet: a run says so in its `panel` field.

## Commands

| Command | Does |
|---|---|
| `plugin.list` | The installed plug-ins, problems reading the folder, the catalogs |
| `plugin.install` | `path`, `data` (base64), `repo` or `catalog` + `id`; `review: true` describes it without installing, `sha256` installs the bundle reviewed |
| `plugin.update` | Checks each plug-in's catalog or repository; `apply` installs updates that ask for no other permissions |
| `plugin.remove` | Removes a plug-in and its settings |
| `plugin.run` | `id`, `command?`, `params?`, `fuel?` |
| `plugin.catalog.list` | The packages WordCraft can run in a catalog, or in every catalog added |
| `plugin.catalog.add` | Adds (or with `remove`, removes) a catalog |
| `addins.get` | Get Add-ins: the list above; from the ribbon, the window |

## What a plug-in may call

Only the commands on the allowlist in `crates/engine/src/plugin.rs` (`ALLOWED`): reading the
document, moving the caret and selecting, typing and deleting, character and paragraph
formatting, styles, lists, tables, page layout, fields, notes, captions, tables of contents,
comments and tracked changes, shapes, text boxes, diagrams, charts, equations, content controls,
Find Next and Replace All. Every other command is refused, so a command added to WordCraft later
stays refused until it is reviewed. Refused, among others:

- files and paths: `file.*` (open, save, export, print, send), pictures and text from files,
  objects and linked or embedded files, templates on disk, fonts, language packs; any parameter
  `path`, `paths`, `file`, `files`, `dir` or `folder` holding text;
- mail merge (`mailings.*`), whose data sources are files;
- other documents and windows, compare and combine, versions, the view (`view.*`, `ui.*`);
- Undo, Redo and Repeat, the clipboard, the Spike, Format Painter;
- macros and Quick Parts;
- preferences: File › Options, AutoCorrect, the custom dictionary, keyboard and ribbon
  customization, the default language, Read Aloud;
- Restrict Editing; plug-ins themselves (`plugin.*`).

A read-only plug-in is refused commands that edit, and a call of its that changed the document
anyway is put back. A call that asks the window for a dialog or a pane is put back and refused
(pass the command's parameters instead).
