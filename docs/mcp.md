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
loopback addresses; see [Keys](control-protocol.md#keys).

## Tracked changes by an agent

Tracked changes and comments are recorded under the session's user name ("WordCraft User" by
default). Name the agent so reviewers can tell its edits from people's: start the headless server
with `wordcraft-cli mcp --author "Claude (copyedit)"`, or run
`file.setAuthor {"name": "Claude (copyedit)"}`. Turn tracking on explicitly with
`review.trackChanges {"value": true}`; without `value` it toggles, which turns tracking *off* in
a document that was saved with it on. `wordcraft-cli run` takes `--author` too.

## Tools

| Tool | What it does |
|---|---|
| `list_commands` | every command (filter with `query`) — over 270 of them |
| `execute` | run one command: `{command: "insert.table", params: {rows: 3, cols: 4}}` |
| `batch` | run several commands in order |
| `new_document` | blank, sample, letter, resume, report |
| `open_document` / `save_document` | docx, doc (open only), odt, rtf, html, md, tex, txt, json; save also pdf, png |
| `type_text` | type at the caret (`paragraphs: true` splits lines into paragraphs) |
| `select_text` | select the n-th occurrence of some text |
| `get_text` / `inspect_document` | read the document back (verify without screenshots) |
| `render_page` | a page as PNG |
| `parity` | feature coverage |
| `screenshot`, `click`, `key`, `ui_inspect` | app only (`--connect`) |
| `chat_join` / `chat_wait` / `chat_send` / `chat_read` / `chat_members` | an invited agent's side of the window's chat (`--connect` only; see [chat.md](chat.md)) |

Resources: `wordcraft://document` (inspect) and `wordcraft://commands`.

As an invited chat agent: `wordcraft-cli mcp --connect 127.0.0.1:7981 --join CODE [--as @name]`
joins with the owner's invite code (or call `chat_join` later). `--as` is optional: the code is
bound to a name. From then on the bridge sends the member key instead of the window key, so every
tool runs as the member, through the chat gate: only commands on the agent allow-list run, and
text edits are tracked under the member's name. `execute`, `batch`, `type_text`, `select_text`,
`get_text`, `inspect_document` and `list_commands` work; `screenshot`, `click`, `key`,
`ui_inspect`, `render_page`, `parity` and the file tools are refused. `chat_wait` returns the
recent history on its first call, then waits at most 25 s per call for new messages.

The acceptance test `crates/mcp/src/tests.rs` writes a formatted document using MCP only.
