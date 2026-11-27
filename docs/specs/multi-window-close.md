# Closing document windows

Specification for #7822 (part of #7671). Terms are defined in [GLOSSARY.md](../../GLOSSARY.md): **Document
window**, **Save prompt**. The decision on the root window is [ADR 0001](../adr/0001-hidden-root-window.md).

## Rules

1. **A close acts on what its window shows.** Closing a document window (its ✕, the OS close, Ctrl+W or
   Ctrl+F4, File › Close) acts only on the document that window shows. No other window changes what it
   shows, and no other window disappears. This holds for the first window opened too.
2. **Another window on the same document.** When another document window still shows the document, the
   window closes without a Save prompt. The Save prompt is asked only when the last window showing a
   document with unsaved changes closes.
3. **The Save prompt belongs to its window.** It appears in the window being closed. While it is up, the
   other windows wait; clicking one of them brings the prompt to the front. Its answer is taken from it
   whichever window is active.
4. **One close at a time.** A second close request made while a Save prompt is up brings that prompt to the
   front and is otherwise ignored, not queued.
5. **Answers.**
   - Save: the document is saved, then the window closes.
   - Don't Save: the window closes without saving.
   - Cancel: nothing closes.
   - Save on a never-saved document opens Save As; cancelling Save As cancels the whole close (or quit).
6. **Quit.** Quit asks about each document with unsaved changes in turn, in a window showing it. Save and
   Don't Save move to the next; Cancel stops the quit and leaves every remaining window as it was.
7. **One document at a time.** On a host that draws one document at a time (the web app, a full-screen
   window, a maximized window on Wayland), ✕, Ctrl+W and File › Close act on the document on screen, and
   the most recently used other document is shown next.
8. **Automation follows the same rules.** Over the control channel and MCP:
   - `file.close` on a document with unsaved changes needs `save: true` (save, then close) or `force: true`
     (close without saving). With neither it closes nothing and returns "unsaved changes: pass save or force".
     A close that rule 2 lets through needs neither.
   - `ui.closeWindow` takes an optional `window` id and otherwise closes the active window, never the root.
   - Automation never leaves a Save prompt waiting for an answer nobody will give.
9. **Nothing gets stuck.** After any answer, cancel, or the prompt's window closing, the app accepts the
   next close or quit. In particular:
   - a second close cannot replace the first and leave its prompt on screen unanswered;
   - clearing the prompt always ends or continues the close, never leaves it half-done;
   - cancelling the prompt works whichever window is active;
   - an answer given while another window is active is still read;
   - an answer over the control channel from another window does not cancel the close.

## Tests

One test per rule and per stuck path; unit tests under 100 ms, app-harness tests under 1 s (#7671).

| Test | Rule |
|---|---|
| Closing the first window of three leaves the other two showing their own documents | 1 |
| Closing a middle window of three closes only its document | 1 |
| ✕ on one of two windows on a modified document asks nothing | 2 |
| ✕ on the last window of a modified document asks, in that window | 2, 3 |
| Clicking another window while the prompt is up raises the prompt | 3 |
| Answering the prompt while another window is active is read | 3, 9 |
| Ctrl+W and ✕ while a prompt is up leave one prompt, and its answer is read | 4, 9 |
| Save, Don't Save and Cancel each do what rule 5 says | 5 |
| Cancelling Save As cancels the close and the quit | 5, 6 |
| Quit with three modified documents asks three times; Cancel on the second keeps two | 6 |
| One-at-a-time host: ✕ closes the document on screen and shows the last used other one | 7 |
| `file.close` without `save`/`force` refuses; with each, saves or discards | 8 |
| `ui.closeWindow` closes the active window, or the one named by `window` | 8 |
| After each of the five stuck paths, a following close and quit both succeed | 9 |
