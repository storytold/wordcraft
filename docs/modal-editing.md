# Modal text editing

On Unix desktop builds, WordCraft can use an installed Neovim for modal document-text editing. Enable **File → Options → General → Modal text editing**, or press **Ctrl+Alt+M**. The status bar shows the current mode and command line. Mode changes do not send desktop notifications.

The editor runs as a separate local process. It uses your normal configuration by default, including Lua mappings; an optional **Editor configuration folder** selects another configuration. A configuration folder must contain `init.lua` or `init.vim`. Your configuration executes with your account permissions, just as when you start the editor yourself. Neovim is not downloaded, bundled or linked into WordCraft. The integration has been exercised with Neovim 0.12.

Only the focused document page sends keys to the modal editor. Dialogs, Find, settings and other text fields keep their ordinary keyboard behaviour. Physical input and framework-injected input take the same document-editing path. Ctrl+Alt+M and the status-bar mode button turn the feature off after queued edits have been applied. F6 and F10 remain available for WordCraft’s region and menu navigation. Native formatting changes resynchronise the text undo baseline; WordCraft’s Undo remains available for its formatting operations.

## Document fidelity

WordCraft remains responsible for the document. The external process receives a text projection of the current story, with paragraph boundaries and placeholders for inline objects. It returns disjoint text edits; the engine applies them transactionally through `text.applyBufferEdits`. It does not export and reimport the document as Markdown or plain text. Untouched character runs, paragraph properties, tables, images, other stories and document metadata stay in the native model.

Counts, operators, motions, text objects, substitutions, search, registers, macros and configuration mappings are interpreted by Neovim itself. A soft line break appears as U+2028 in its text projection. Changes associated with an observed editor undo state share one native undo step; Undo and Redo restore rich native snapshots. Save and native editing commands wait for queued modal text before running. `:w` requests WordCraft's ordinary Save operation, and `:w path` requests Save As. Configuration plug-ins that write the buffer, including autosave, request those same saves. Use WordCraft's file commands for opening and closing documents; a quit request stops modal editing without discarding pending document text.

## Explicit limits

This integrates a text-editing engine, not its terminal or graphical interface. Plug-ins that open their own buffers, floating windows, terminal panes or file explorers are not a WordCraft interface. The bridge stops if another editor buffer becomes active. Opening and closing documents, working on embedded objects and changing document layout remain WordCraft operations.

Edits that would cross table-cell boundaries, replace inline-object placeholders or fail native document protection are refused atomically. Intermediate undo states that were not observed by WordCraft are refused rather than reconstructed with lost formatting. A story is limited to 2 MiB and input queues are bounded. An unresponsive editor times out; a late reply cannot overwrite a newer document revision. A refused edit stops modal editing and explains the reason in WordCraft's status bar.

## Automation

`ui.modalEditing {"value": true, "configDirectory": "/absolute/config/folder"}` enables the same feature as Options. Omit `configDirectory` to retain the saved setting; an empty string uses the editor's normal configuration. `{"keys": "gg0ciwChanged<Esc>"}` queues key notation, and `{"inspect": true}` reports mode, pending input and any error. A queued result acknowledges input, not completion; inspect the document before depending on the edit.

`text.applyBufferEdits` takes `story`, the exact `before` projection, ordered disjoint `edits` with UTF-8 byte offsets, and final `anchor`/`focus` byte offsets. Stale text, invalid character boundaries and unsafe structural changes leave document content and undo history unchanged.

Implementation references: [Neovim's documented remote interface](https://neovim.io/doc/user/remote.html), [API](https://neovim.io/doc/user/api.html), and [Lua diff API](https://neovim.io/doc/user/lua.html#vim.diff()). No editor implementation source was used.
