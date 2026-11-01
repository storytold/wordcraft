# GeenioAI, the AI assistant

> **Last reviewed:** 2026-10-20 · **Last updated:** 2026-10-20 · **Change:** minor (review fixes: file commands ask first, `ask` confines the agent's files, the old key file never outlives a connection; previously major: the desktop app runs GeenioAI on the shared Craft AI layer: the account is a layer connection, its key in the system keychain, an agreement before anything is sent; previously minor: streaming replies, the interface strings in all 18 languages) · **Target:** Microsoft Word (Microsoft 365) Copilot pane and margin icon

WordCraft's answer to Word's Copilot: a chat pane and a margin icon whose agent writes and edits
the open document with WordCraft's own commands. Any model can sit behind it: the desktop app and
the CLI reach models through a [FANTASTICO](https://fantastico.cc) account (one prepaid credit,
every major LLM: Claude, GPT, Gemini, Llama…), and the model is chosen in the pane's settings.

## What the user sees

| Where | What | Word |
|---|---|---|
| Home › GeenioAI | Opens and closes the pane (`ui.assistant`) | Home › Copilot |
| The pane (right) | Chat with the assistant; suggestions until the conversation starts (Summarize this document, Suggest a title, Check the tone); a status line in plain words while it works ("Writing…", "Running Bold…"); a confirmation card when it wants to start a new document, open, save or export a file (also through a `file.*` command it runs); the credit, a fresh start (behind a confirmation) and the settings in the header | Copilot pane |
| The margin icon | Beside the paragraph the caret is in (the selection's first line when there is one), in the left margin; with that line scrolled out of view it waits at the top or bottom edge of the page area, so it is always there to click. A click opens a prompt box ("Describe what you'd like to write" on an empty paragraph, Word's Draft with Copilot) and the quick actions that fit: Rewrite the selection, Make it shorter, Fix spelling and grammar, Summarize the selection; Rewrite this paragraph; Open GeenioAI | Copilot icon in the margin |
| Settings (gear) | Reply language (automatic, the interface language, English); the model, from the account's list with a search box; the account and its credit; change the account key | — |

Every request goes with where the user is in the document: the selected text (which `type_text`
replaces), an empty paragraph the caret is on, or the paragraph around the caret. The assistant
rewrites in place, drafts at the caret, and answers questions in the chat. **One turn is one undo
step**: Undo takes back everything a request changed.

## Keys and privacy

- The onboarding asks for a FANTASTICO **account key** (`ck-fant-…`). The desktop app checks it
  with the account service, then saves the account as a connection of the shared **Craft AI layer**
  ([craft-ai-integrations](https://github.com/storytold/craft-ai-integrations)) with the key in the
  **system keychain** (Credential Manager, Keychain, Secret Service). An account connected in the
  Craft launcher or another Craft app works here without a key, and one connected here works there.
  Nothing in the interface can read the key back: the pane can store one or replace it
  (Settings › Change account key), never show it. No command, control method or log line carries it.
  A key an older WordCraft kept in `<settings>/assistant/fantastico.key` moves into the keychain
  the first time GeenioAI needs it, and the file is removed; when a connection exists already the
  file is an unused copy and is removed too. A checked new key replaces it.
- **Nothing is sent before the user agrees.** Before the first request, a card in the pane says
  what goes where (the requests, the document text the assistant reads and what its commands
  return, to FANTASTICO and the provider it routes to) and asks to allow it for WordCraft. The
  layer keeps the agreement per app and connection, and refuses a request without it.
- Prompts and the document text go straight to the model provider: FANTASTICO mints a
  short-lived, capped runtime key that only the provider accepts, so a request that went through
  FANTASTICO instead could not be answered, and FANTASTICO never sees the prompts. The
  layer's job log records each request's model and sizes, never its text.
- Runs without preferences (`WORDCRAFT_NO_PREFS`, agents' test runs) have no assistant and never
  touch the key. The web build says the assistant isn't available yet (no host).

## For agents and scripts

- `ui.assistant {"value"?: bool}` shows or hides the pane (toggles without `value`); the result
  says whether a host is attached (`available`) and a key is stored (`connected`).
- `ui.assistant.ask {"prompt": "…", "draft"?: bool}` opens the pane and sends the prompt with the
  document context; `draft` asks for the answer in the document, as the margin icon's box does.
- `wordcraft-cli ask "PROMPT" [--file F | --template T] [--model M] [--max-steps N] [--yes]
  [--author NAME] [--save OUT] [--automation-read-root DIR] [--automation-write-root DIR]` runs the
  same agent headlessly on a document, with the key in `$FANTASTICO_KEY` (an account key, or a
  project key `sk-fant-…` for a trusted backend). The reply goes to stdout, progress to stderr;
  `--yes` allows new/open/save without asking. The agent reads and writes no file of its own unless
  the automation roots allow it (as for `wordcraft-cli mcp`); `--file` and `--save` are the CLI's
  own and are not limited, and `--save` is checked before the model is asked.

## How it works

| Layer | Crate | Job |
|---|---|---|
| L5 | `wordcraft-assistant` | The agent loop: the model asks for tools, they run against any `wordcraft_mcp::Backend` (a headless session or the app), their results go back until the model answers. No network code: the host brings the model as a `Provider` (OpenAI-style chat completions). Model output is hostile input: at most 20 steps a turn and 16 tool calls a reply, cut results, malformed arguments and unknown tools answered with errors, old turns forgotten whole. Out of steps, the model gets one last call without tools to say what it did |
| L6 | `wordcraft-ui-egui::assistant` | The pane and the margin icon. The loop runs on a worker thread; its tools reach the document through the control methods, run on the UI thread each frame (`tick`), joined into one undo step per turn |
| L5 (native) | `wordcraft-fantastico` | A FANTASTICO account as a streaming `Provider`: a transport of its own under `fantastico-sdk` reads the provider's server-sent events for a `stream: true` request, hands every piece of text on, and gives the SDK the same JSON a whole reply would carry (keys, retries and error classification stay the SDK's). Native only (`Class::Native` in the layer table): the web build never sees it |
| app | `apps/wordcraft/src/assistant_host.rs` | The host on the Craft AI layer (`craft-ai-client`): GeenioAI is a tool (`geenio-chat`) bound to a FANTASTICO connection and model in the shared store; each model call is a layer job run by the `craft-ai-fantastico` adapter, which carries the whole conversation (earlier replies and tool results as turns) and streams the reply. The `fantastico-sdk` client reads the account and its credit. `wordcraft-cli ask` still uses `wordcraft-fantastico` with `$FANTASTICO_KEY` |

The tools offered to the model are the command-level MCP tools: `get_text`, `inspect_document`,
`select_text`, `type_text`, `execute`, `batch`, `list_commands`, and, behind a confirmation,
`new_document`, `open_document`, `save_document`. A `file.*` command run through `execute` or
`batch` (open, save, export…) asks the same confirmation, and the interface's own `ui.*` commands
are refused. Pictures and pointer tools stay out.

Replies stream: the pane shows the model's text as it arrives, in a bubble that fills up (`Provider::complete_streaming`, `Delta`); `wordcraft-cli ask` prints it as it comes. An endpoint that can't stream answers whole.

## Known gaps

- No tracked changes for the assistant's edits (Word's Copilot rewrites offer a Keep/Discard
  preview); Undo takes a turn back in one go.
- No "Visualize as a table" or other Copilot-specific actions beyond the ones above.
- The margin icon is drawn only for body text (not in headers, footers, notes, text boxes or
  tables with whole cells selected).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-20 | minor | Review fixes: file commands through `execute`/`batch` ask first and `ui.*` commands are refused; `wordcraft-cli ask` confines the agent's files (automation roots, none by default) and checks `--save` first; the old key file never outlives a connection |
| 2026-10-20 | major | The desktop host moves onto the Craft AI layer: shared connection, key in the system keychain, agreement card before the first request, the old key file migrated |
| 2026-10-18 | minor | Streaming replies (`wordcraft-fantastico`, `Provider::complete_streaming`); the interface strings translated in every catalog |
| 2026-10-18 | major | First version: pane, margin icon, FANTASTICO host, `wordcraft-cli ask` |
