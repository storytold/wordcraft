# Zotero

WordCraft works with the [Zotero](https://www.zotero.org) desktop app the way Word does: Zotero's
own citation dialog, all of its citation styles, automatic bibliography and refresh. Nothing to
install in Zotero; it only has to be running. The **Zotero** tab in the ribbon has the buttons:
Add/Edit Citation, Add Note, Add/Edit Bibliography, Refresh, Document Preferences and Unlink
Citations (each runs one of the commands below).

## Commands

| Command | What it does |
|---|---|
| `ui.zotero.addEditCitation` | Add a citation at the caret, or edit the one the caret is in |
| `ui.zotero.addEditBibliography` | Add the bibliography at the caret, or edit it |
| `ui.zotero.refresh` | Update every citation and the bibliography |
| `ui.zotero.setDocPrefs` | Citation style, language, footnotes or in-text |
| `ui.zotero.removeCodes` | Unlink citations: keep the text, drop Zotero's codes |
| `ui.zotero.addNote` | Insert a Zotero note |
| `ui.zotero.status` | `{busy, waitingForAnswer, last}` |
| `ui.zotero.answer` | `{"button": n}`: answer a question Zotero is asking (OK/Yes 1, No/Cancel 0; with Yes/No/Cancel: Yes 2, No 1, Cancel 0) |
| `ui.zotero.cancel` | Stop the command in flight: WordCraft hangs up on Zotero and the Zotero buttons work again. Edits already made stay (one undo step) |

They run in the app (and through the control channel and MCP bridged to the app). A command
returns at once with `{"started": …}`; Zotero then works in its own window and WordCraft answers
it in the background. All changes from one command are one undo step. If New or Open replaces
the document while a command is running, Zotero's remaining calls for it get an error and the
new document is left as it is.

From a terminal, against a file:

```
wordcraft-cli zotero refresh paper.docx --save paper.docx [--trace]
```

Two engine commands work with Zotero's fields without Zotero running:

- `caret.pastCitation` (Zotero › Move Past Citation) moves the caret to just after the citation
  it is in or before. Adjacent citations count as one. If there is none, it moves past the next
  citation.
- Read Aloud (`review.readAloud`, Review › Speech, Mod+Alt+Space) skips citations and the
  bibliography by default. This covers Zotero, Mendeley, EndNote and Word's own citation
  fields. The player window has previous/next sentence, pause, stop and speed (0.5–3×).
  Its commands are `readAloud.playPause`, `.next`, `.previous`, `.stop`, `.speed {"value"}`,
  `.skipCitations {"value"}` and `.status`. Set `WORDCRAFT_SPEECH=off` to keep it silent.

Switching between an in-text style and a note style in Document Preferences moves the citations:
into new footnotes (or endnotes) at the same place, and back into the text where the note marks
were; a note that held only the citation is removed.

## Word compatibility

Citations are stored as Word's Zotero plugin stores them — fields with the code
`ADDIN ZOTERO_ITEM CSL_CITATION {…}`, the bibliography as `ADDIN ZOTERO_BIBL … CSL_BIBLIOGRAPHY`,
document preferences in the custom properties `ZOTERO_PREF_1…n` — so one document can be edited
in Word and WordCraft in turn, with Zotero working in both. (Zotero talks to WordCraft over the
protocol it uses for LibreOffice and calls the field type "Reference Mark" there; WordCraft keeps
Word's "Field" in the file.)

## Limits

- Zotero's "Bookmarks" field type isn't supported; keep the default.
- Zotero's document transfer (Export/Import Document) isn't needed and isn't supported.
- The web version can't reach Zotero (browsers can't open the socket).

## Session limits

Whatever listens on Zotero's port can't keep WordCraft waiting or growing forever. A command ends
with an error (shown in the status bar and in `ui.zotero.status`'s `last`) when:

| Limit | Default | Why |
|---|---|---|
| No message from Zotero | 60 min | Zotero's dialogs wait for the user, so this is long; it only ends a session Zotero abandoned |
| One message, once it starts arriving | 60 s | a peer that sends a message a byte at a time |
| One reply, being sent | 60 s | a peer that stops reading |
| The whole command | 4 h | dialogs included |
| Messages in one command | 200,000 | a peer that calls forever |
| One message | 32 MiB | |

While a command runs the status bar says "Waiting for Zotero…" with a **Cancel** link
(`ui.zotero.cancel`), which ends it at any time, even with no document open. A command's result
keeps its first 20 failed calls (each cut to 500 bytes) and counts the rest (`moreErrors`); the
app's log names the first 200 calls (each name cut to 128 bytes) and the first 20 failures, and
counts the rest. A question from Zotero is shown up to 2,000 bytes. These are the fields of `client::Options` for the CLI and tests.

## Who can drive the document

Zotero's protocol has no handshake that proves the program on port 23116 is Zotero, and that
program can read the citations and edit the open document. WordCraft limits who that can be.
In short: **every system** gets the loopback rule; **only Linux** also checks that the listener
isn't another user's. On Windows and macOS (and the BSDs) the loopback rule is the only check.

- Every system: it only connects to this computer (a loopback address such as `127.0.0.1`);
  any other address is refused before connecting.
- Linux only: it checks, in the kernel's socket table (`/proc/net/tcp`, `/proc/net/tcp6`), that the
  program listening on the port runs as the same user as WordCraft, before sending anything. If
  another account on a shared machine took the port while Zotero wasn't running, WordCraft
  refuses with an error and the document isn't touched. When the table doesn't show the
  listener (WSL2 mirrored networking with Zotero running on Windows, a local proxy), its owner
  is unknown and only the loopback rule applies, as below. That holds only when the tables were
  read in full: a read error, or an open error other than "not found", **refuses**. Sandboxes
  that deny `/proc/net/tcp` (some AppArmor or Snap profiles, for example) are therefore refused.
- Windows, macOS and the BSDs: **no same-user check.** They have no such table to read
  without OS calls WordCraft doesn't make (it uses no `unsafe` code), so only the loopback rule
  applies there (and on Linux when the listener isn't in the table). Remaining risk: on a
  computer shared with other accounts, a program another user leaves on port 23116 while
  Zotero is closed would be taken for Zotero. An owner check for these systems is tracked in
  #3831. A program running as you is trusted on every system; it could change your documents
  anyway.
- `wordcraft-cli zotero` prints Zotero's messages (and `--trace` lines) with control
  characters, line breaks and backslashes shown as escapes (`\n`, `\u{1b}`, `\\`), so a peer
  can't forge other output or drive the terminal.

## How it works

`crates/zotero` implements Zotero's published word-processor wire protocol (port 23116):
`client` runs a session, `Bridge` answers Zotero's `Document_*` / `Field_*` calls against the
session. The app (`crates/ui-egui/src/zotero.rs`) runs the socket on a thread and answers on the
UI thread.
