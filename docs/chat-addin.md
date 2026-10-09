# WordCraft Chat: quick guide

The Chat add-in lets you invite AI agents into the document you have open. They talk with you in
a chat pane and edit the document as themselves: every text change they make is a tracked change
under their own name.

## Start
1. Start WordCraft with its control channel: `wordcraft --control 7981`. The port listens on
   `127.0.0.1` only and every request needs a key (see `control-protocol.md`).
2. Open your document.
3. Open the **Chat** pane: **Review › Chat**, or **Share › Invite an agent…**.

The chat is in English. Set `WORDCRAFT_CHAT_LANG=pt` before starting WordCraft for Portuguese.

Agents use the `wordcraft-chat` client (`tools/wordcraft-chat/`; `install.sh` installs it as
`~/.local/bin/wordcraft-chat`).

## Invite an agent
1. In the **You:** field, type your name. It cannot start with @.
2. In the agent name field (hint `@claude`), type the agent's name, for example `@claude`.
3. Click **Invite**. The line `Join the WordCraft chat: wordcraft-chat join <window>:<code> --as @name`
   appears. The name in the line is always the one bound to the code, even if you type something
   else in the field afterwards.
4. Click **Copy**.
5. Paste the line into the agent's session. The agent joins the chat. The line goes away when the
   agent joins or when the code expires (10 minutes).

## Write in the chat
Type in the box "Message (@claude …), Enter to send". **Enter** sends. **Shift+Enter** starts a new
line.
- With only one agent in the chat, every line you write is for it: you do not need `@claude`.
- With two or more agents, mention the one that should act (`@claude`) or use `@all`. A line without
  a mention is conversation.
- `@owner`, `@all`, `@you` and `@me` (and the Portuguese `@dono`, `@todos`, `@ti`, `@eu`) are not
  agent names. Agents see your lines as `OWNER #n [@you]: …` when they are for them.

## What an agent can do
- Write and delete text in the document, headers, footers and notes. Every text change is ALWAYS a
  tracked change under its name, and you can reject it. It cannot write or delete in comments by
  others (comment text is shown without markup).
- Simple formatting, not tracked: bold, italic, underline, strike, font and size, colour,
  highlight, subscript and superscript; alignment, spacing, indents and paragraph style. Each one
  shows in the chat as a system line: `@claude formatted: format.bold`.
- In a new paragraph of its own (only its tracked text) it may set paragraph formatting and lists.
- Comment and reply. It changes and deletes only its own comments.
- Resolve a comment: the chat shows `@claude resolved the comment by <author>`.
- Accept and reject changes, also only part of a change. Whenever that changes the document, the
  chat shows a line with the characters accepted or rejected and their authors, counting the text,
  headers, footers, notes and paragraph marks (a mark counts one):
  `@claude rejected 2 characters from Ann`. No change, no line.
- Use your selection: when you say "this" or "the selected text", the agent starts with
  `select.owner`. It gets a copy of your selection (with no selection, the paragraph at your caret).
  Your selection stays where it is.

## What an agent cannot do
WordCraft checks EVERY command of an agent, also the ones that should not change the document. A
change that is not tracked text or formatting from the list above is undone and refused with
`untracked change refused`. For example:
- In your paragraphs: add or remove lists and numbering, change the list level, borders, shading,
  tabs, page break before, keep with next, keep lines together, outline level, right-to-left
  direction. The paragraph mark (it formats the list number or bullet) only changes with simple,
  announced formatting. Not through an Enter it types at the start or in the middle of your
  paragraph either: both halves are still yours.
- Change the text of comments by others (not even as a tracked change). It may reply to them.
- Write list label text: define numbers or bullets (`para.defineNumber`, `para.defineBullet`, a
  custom bullet in `para.bullets`), change the start number (`para.setNumberingValue`,
  `para.restartNumbering`). Change list definitions that already exist.
- Reject new paragraphs (WordCraft does not join paragraphs on reject). The agent gets
  `rejecting new paragraphs: ask the OWNER`. You reject them (**Review › Reject**).
- Insert or delete tables, rows and columns; format tables.
- Change the page (margins, size, orientation, sections, columns), the design, the theme or the
  style definitions.
- Insert footnotes, fields, indexes, links, bookmarks, shapes, text boxes. Create a new header or
  footer (it can write in one of yours, tracked).
- Replace all (`edit.replaceAll`), hide text, sort paragraphs, change case.
- Put revision marks in someone else's name.
- Open, save or delete files. Use the clipboard.
- Undo or redo. Change the document protection.
- Macros, mail merge, the dictionary, Quick Parts.
- Touch the window or dialogs.
- Write into a selection that you or another agent changed meanwhile: it gets
  `your selection changed; select again` and selects again.

## Your side
- Your caret and selection stay where they were and follow text an agent adds before them. If an
  agent replaces the text you have selected, your selection stays on that text (now struck through).
  Your view and your macros are untouched.
- Undo is yours: **Ctrl+Z** also undoes agents' edits. The usual way to refuse an agent's edit is to
  reject it (**Review › Reject**).
- With the window minimized or on another workspace, agents' commands on the document wait. After
  55 seconds they are dropped ("expired") and never run later. Agents' chat messages still arrive.

## Remove an agent
In the member list, click **Remove** next to its name. The agent's `listen` ends and it gets
"SYSTEM: you were removed from the chat".

## Brake
After 8 agent messages in a row, the line "Agents wait for you (8 messages in a row)." appears.
Write something to let them go on.

## Where the conversation is kept
In a file next to the document: `<document>.chat.jsonl`. Never inside the document. Before the
document has a file, the pane shows "The chat is kept in memory until you save the document."
Save the document to keep the conversation.
- **Save As**: the conversation goes on and moves to the file with the new name.
- **Open** or **New**: the chat switches to the other document's conversation (only its own). The
  line `document: <name>` (or `document: new`) appears. Agents stay in the chat; that document's
  old orders reach them as history, never as new orders.
- **Restore a version** (File › Info › Version History) is the same document: the conversation stays.
- A folder WordCraft cannot see: in a Flatpak, Save As to a folder outside the app's folders goes
  through the document portal (`/run/flatpak/doc/…`, on the host `/run/user/…/doc/…`) and only the
  document reaches the folder. The conversation is then kept in
  `<config>/wordcraft/chat-logs/<id>-<name>.chat.jsonl` and the chat shows
  `chat saved in <path>`. The same happens for any folder where the chat file cannot be created.
- If even that file cannot be read or written, the pane shows the error in red
  ("Chat log not saved: …"). The error stays until the conversation can be kept in a file (for
  example after another Save As).
